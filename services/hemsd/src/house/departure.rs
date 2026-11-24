//! A household requests outlet energy by departure from one virtual OCPP 1.6J wallbox.
mod grid;
mod peer;

use anyhow::{Context, Result, ensure};
use ems_drv::ocpp::{DateTime, outlet_register_wh};
use ocpp_kit::{
    RawValue, Version,
    engine::IncomingRequest,
    rpc::CallError,
    transport::{
        Admission, Auth, AuthOutcome, BasicAuthPassword, BoxFuture, Credentials, Csms, Ctx, Handler,
    },
    v1_6::{
        AuthorizationStatus, BootNotificationResponse, CsRequest, IdTagInfo, MeterValuesResponse,
        RegistrationStatus, StartTransactionResponse, StopTransactionResponse,
    },
};
use std::{
    fmt,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, time::timeout};

const CONNECTOR: i32 = 1;
const TX: i32 = 1;
const MAX_W: i32 = 11000;
const STATION: &str = "Home-01";

#[derive(Clone, Copy)]
struct Times {
    start: DateTime,
    resume: DateTime,
    leave: DateTime,
}

#[derive(Default)]
struct Meter {
    start: Option<i32>,
    resume: Option<i32>,
    stop: Option<i32>,
}

#[derive(Clone, Copy, PartialEq)]
enum Visit {
    Arrival,
    Return,
}

fn within_quarter(previous: i32, current: i32) -> bool {
    (0..=i64::from(MAX_W / 4)).contains(&(i64::from(current) - i64::from(previous)))
}

struct Controller {
    times: Times,
    meter: Arc<Mutex<Meter>>,
}

impl Handler for Controller {
    fn on_request(
        &self,
        ctx: Ctx,
        req: IncomingRequest,
    ) -> BoxFuture<'_, Result<Box<RawValue>, CallError>> {
        Box::pin(async move {
            let visit = ctx
                .session::<Visit>()
                .ok_or_else(|| CallError::security("unrecognized wallbox session"))?;
            let request = ctx.decode::<CsRequest>(&req)?;
            let mut meter = self.meter.lock().expect("departure meter");
            match request {
                CsRequest::BootNotification(_) => ctx.reply(&BootNotificationResponse::new(
                    RegistrationStatus::Accepted,
                    if *visit == Visit::Return {
                        self.times.resume
                    } else {
                        self.times.start
                    },
                    300,
                )),
                CsRequest::StartTransaction(r)
                    if *visit == Visit::Arrival
                        && meter.start.is_none_or(|value| value == r.meter_start)
                        && r.connector_id == CONNECTOR
                        && r.timestamp == self.times.start
                        && r.meter_start >= 0 =>
                {
                    meter.start = Some(r.meter_start);
                    ctx.reply(&StartTransactionResponse::new(
                        IdTagInfo::new(AuthorizationStatus::Accepted),
                        TX,
                    ))
                }
                CsRequest::MeterValues(r) if *visit == Visit::Return => {
                    let value = outlet_register_wh(&r, CONNECTOR, TX, self.times.resume);
                    if let (Some(start), Some(wh)) = (meter.start, value)
                        && within_quarter(start, wh)
                        && meter.resume.is_none_or(|previous| previous == wh)
                    {
                        meter.resume = Some(wh);
                    }
                    ctx.reply(&MeterValuesResponse::new())
                }
                CsRequest::StopTransaction(r)
                    if *visit == Visit::Return
                        && meter.stop.is_none_or(|previous| previous == r.meter_stop)
                        && r.transaction_id == TX
                        && r.timestamp == self.times.leave
                        && meter
                            .resume
                            .is_some_and(|v| within_quarter(v, r.meter_stop)) =>
                {
                    meter.stop = Some(r.meter_stop);
                    ctx.reply(&StopTransactionResponse::new())
                }
                CsRequest::StartTransaction(_) | CsRequest::StopTransaction(_) => {
                    Err(CallError::new(
                        ocpp_kit::rpc::ErrorCode::PropertyConstraintViolation,
                        "transaction boundary does not match this session",
                    ))
                }
                other => Err(CallError::not_supported(other.action().as_str())),
            }
        })
    }
}

/// Outlet readings for one completed transaction across its two socket sessions.
#[derive(Debug)]
pub struct Receipt {
    grid: grid::Balance,
    departure: time::OffsetDateTime,
    goal: i32,
    start: i32,
    resume: i32,
    stop: Option<i32>,
}

impl fmt::Display for Receipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.outlet(f)?;
        write!(
            f,
            "\nConnection meter · {} Wh imported · {} Wh exported · {} Wh imported while charging paused",
            self.grid.import, self.grid.export, self.grid.interruption
        )
    }
}

impl Receipt {
    /// Send the simulated outlet evidence as a site-signed CloudEvent.
    ///
    /// # Errors
    /// Returns an error for insecure transport, rejected evidence or failed delivery.
    pub async fn publish(&self, origin: &str, site: &str, secret: &[u8]) -> Result<()> {
        ensure!(
            !secret.is_empty(),
            "a departure receipt needs a site signing key"
        );
        let endpoint = format!("{}/v1/departures", origin.trim_end_matches('/'));
        crate::report::is_confidential(&endpoint)?;
        let evidence = ems_events::departure::SimulatedDeparture {
            departure: self.departure,
            requested_wh: self.goal,
            opening_wh: self.start,
            rejoin_wh: self.resume,
            closing_wh: self.stop,
        };
        let now = time::OffsetDateTime::now_utc();
        let event = ems_events::Event::new(
            ems_events::SITE_DEPARTURE_REHEARSED,
            format!("ems://sites/{site}"),
            format!(
                "{site}:departure:{}:{}",
                self.departure.unix_timestamp(),
                self.stop.is_some()
            ),
            self.departure,
            evidence,
        );
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()?;
        crate::report::deliver(
            &client,
            &endpoint,
            secret,
            &event.id,
            &event.to_bytes()?,
            now,
        )
        .await?;
        Ok(())
    }

    fn outlet(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(stop) = self.stop else {
            writeln!(
                f,
                "Unverified at 07:00 UTC · {} Wh requested · departure register missing",
                self.goal
            )?;
            return write!(
                f,
                "Last reading at 06:45 UTC · {} Wh delivered by rejoin ({} → {} Wh)",
                self.resume - self.start,
                self.start,
                self.resume
            );
        };
        let delivered = stop - self.start;
        let missing = (self.goal - delivered).max(0);
        let outcome = if missing == 0 {
            "Goal met"
        } else {
            "Goal missed"
        };
        writeln!(
            f,
            "{outcome} at 07:00 UTC · {delivered} Wh delivered of {} Wh requested · {missing} Wh short",
            self.goal
        )?;
        write!(
            f,
            "Outlet register (UTC)  06:15 {} Wh  →  06:45 reconnected {} Wh  →  07:00 {} Wh",
            self.start, self.resume, stop
        )
    }
}

fn admit_station(
    identity: &str,
    credentials: &Credentials,
    expected: &BasicAuthPassword,
    meter: &Mutex<Meter>,
) -> AuthOutcome {
    if identity != STATION
        || !matches!(credentials, Credentials::Basic { password, .. } if expected.verify(password))
    {
        return AuthOutcome::Reject;
    }
    let Ok(meter) = meter.lock() else {
        return AuthOutcome::Reject;
    };
    let visit = if meter.start.is_some() {
        Visit::Return
    } else {
        Visit::Arrival
    };
    Admission::new().context(visit).into()
}

/// Rehearse a 45-minute session with a 15-minute interruption and an 11 kW wallbox.
///
/// # Errors
/// Returns an error for an invalid goal, a failed local protocol exchange, or missing rejoin evidence.
pub async fn rehearse(goal: i32, missing_close: bool) -> Result<Receipt> {
    ensure!((1..=10000).contains(&goal), "goal must be 1–10000 Wh");
    let times = Times {
        start: DateTime::parse("2026-01-15T06:15:00Z")?,
        resume: DateTime::parse("2026-01-15T06:45:00Z")?,
        leave: DateTime::parse("2026-01-15T07:00:00Z")?,
    };
    let meter = Arc::<Mutex<Meter>>::default();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let password = BasicAuthPassword::generate(Version::V1_6)?;
    let expected = password.clone();
    let admission_meter = Arc::clone(&meter);
    let csms = Csms::builder()
        .bind(addr)
        .versions([Version::V1_6])
        .max_connections(1)
        .supersede_existing(false)
        .authenticate(move |auth: Auth| {
            std::future::ready(admit_station(
                auth.identity.as_str(),
                &auth.credentials,
                &expected,
                &admission_meter,
            ))
        })
        .handler(Controller {
            times,
            meter: Arc::clone(&meter),
        })
        .build()?;
    let central = csms.handle();
    let exercise = peer::drive(&central, addr, password, times, goal, &meter, missing_close);
    let grid = tokio::select! {
        r = csms.serve_on(listener) => anyhow::bail!("local controller ended: {r:?}"),
        r = timeout(Duration::from_secs(10), exercise) => r.context("household OCPP exchange timed out")??,
    };
    let meter = meter
        .lock()
        .map_err(|_| anyhow::anyhow!("departure meter unavailable"))?;
    Ok(Receipt {
        grid,
        departure: time::OffsetDateTime::from_unix_timestamp(times.leave.timestamp().as_second())?,
        goal,
        start: meter.start.context("missing plug-in register")?,
        resume: meter.resume.context("missing rejoin register")?,
        stop: meter.stop,
    })
}

fn household() -> Result<crate::site::Household> {
    use crate::HouseholdConfig;
    let mut config = HouseholdConfig {
        battery: None,
        pv: None,
        heat_pump: None,
        dhw: None,
        dishwasher: None,
        ..HouseholdConfig::default()
    };
    config
        .evse
        .as_mut()
        .context("reference wallbox missing")?
        .switchable = false;
    crate::site::Household::build(&config)
}

fn allowed_watts(
    home: &crate::site::Household,
    at: DateTime,
    requested: i32,
    grid: ems_core::prelude::Measurement,
) -> Result<i32> {
    use ems_core::prelude::{AssetTarget, Horizon, Measurement, Plan, Power, Slot, SlotPlan};
    use ems_realtime::{
        Arbiter, ArbiterConfig, Tick,
        guard::{GridLimits, SiteState},
    };
    use rust_decimal::prelude::ToPrimitive;
    use std::collections::BTreeMap;
    let now = time::OffsetDateTime::from_unix_timestamp(at.timestamp().as_second())?;
    let outlet = home.evse.as_ref().context("missing household outlet")?;
    let state = SiteState {
        grid: Some(grid),
        assets: BTreeMap::from([(outlet.clone(), Measurement::power(now, Power::ZERO))]),
        ..SiteState::default()
    };
    let mut plan = Plan::empty(Horizon::new(now, 1), now);
    plan.slots.push(SlotPlan {
        slot: Slot::containing(now),
        targets: vec![AssetTarget::fixed(
            outlet.clone(),
            Power::new(f64::from(requested)),
        )],
        marginal_eur_per_kwh: None,
        flexibility_eur_per_kwh: None,
    });
    let decision = crate::runtime::control::decide(
        &Arbiter::new(ArbiterConfig::default()),
        Tick {
            now,
            site: &home.site,
            state: &state,
            limits: &GridLimits::default(),
            plan: Some(&plan),
            cem: &BTreeMap::new(),
            overrides: &BTreeMap::new(),
            previous: &BTreeMap::new(),
            delivered: &BTreeMap::new(),
            phases: &BTreeMap::new(),
        },
        None,
    );
    decision
        .commanded
        .get(outlet)
        .and_then(|p| p.get().to_i32())
        .context("gateway did not authorize an outlet target")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_provisioned_station_can_resume_the_transaction() {
        let expected = BasicAuthPassword::generate(Version::V1_6).unwrap();
        let provided = Credentials::Basic {
            user: STATION.into(),
            password: expected.as_bytes().to_vec().into(),
        };
        let wrong = BasicAuthPassword::generate(Version::V1_6).unwrap();
        let meter = Mutex::new(Meter::default());
        assert!(matches!(
            admit_station("Other", &provided, &expected, &meter),
            AuthOutcome::Reject
        ));
        assert!(matches!(
            admit_station(STATION, &provided, &wrong, &meter),
            AuthOutcome::Reject
        ));
        assert!(matches!(
            admit_station(STATION, &Credentials::None, &expected, &meter),
            AuthOutcome::Reject
        ));
    }

    #[tokio::test]
    async fn departure_is_closed_from_the_returning_wallbox_register() {
        let (met, missed, unknown) = tokio::try_join!(
            rehearse(3600, false),
            rehearse(6000, false),
            rehearse(3600, true)
        )
        .unwrap();
        assert!(met.grid.interruption > 0);
        assert!(met.grid.import > 3600);
        assert_eq!(met.grid.export, 0);
        assert_eq!(missed.grid.import - met.grid.import, 4750 - 3600);
        assert_eq!(unknown.grid.import, met.grid.import);
        assert_eq!(unknown.grid.interruption, met.grid.interruption);
        assert!(unknown.stop.is_none());
        assert!(unknown.to_string().contains("Unverified"));
        assert_eq!(
            (met.start, met.resume, met.stop),
            (48210, 49410, Some(51810))
        );
        assert_eq!(
            (missed.start, missed.resume, missed.stop),
            (48210, 50210, Some(52960))
        );
        assert!(met.to_string().contains("3600 Wh delivered"));
        assert!(missed.to_string().contains("1250 Wh short"));
        assert!(!within_quarter(4000, 3999));
        assert!(within_quarter(4000, 4000));
        assert!(!within_quarter(0, i32::MAX));
    }
}
