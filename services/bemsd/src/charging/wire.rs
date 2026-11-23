//! A bounded OCPP 1.6J commissioning replay. Scenario times are logical clock ticks.
use super::{Outcome, Received, Recording, Script, StationScript, Trace};
use anyhow::{Context, Result, ensure};
use ocpp_kit::types::{DateTime, Decimal};
use ocpp_kit::{
    RawValue, Version,
    engine::IncomingRequest,
    rpc::CallError,
    transport::{
        Admission, Auth, AuthOutcome, BasicAuthPassword, BoxFuture, Credentials, Csms, CsmsHandle,
        Ctx, Handle, Handler, Station as ChargePoint,
    },
    v1_6::{
        BootNotificationRequest, BootNotificationResponse, ChargingProfile, ChargingProfileKind,
        ChargingProfilePurpose, ChargingProfileStatus, ChargingRateUnit, ChargingSchedule,
        ChargingSchedulePeriod, CsRequest, CsmsRequest, Measurand, MeterValuesRequest,
        MeterValuesResponse, RegistrationStatus, SetChargingProfileRequest,
        SetChargingProfileResponse, UnitOfMeasure,
    },
};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};

type Inbox = mpsc::Sender<MeterValuesRequest>;

struct CentralSystem(DateTime);

impl Handler for CentralSystem {
    fn on_request(
        &self,
        ctx: Ctx,
        req: IncomingRequest,
    ) -> BoxFuture<'_, Result<Box<RawValue>, CallError>> {
        Box::pin(async move {
            match ctx.decode::<CsRequest>(&req)? {
                CsRequest::BootNotification(_) => ctx.reply(&BootNotificationResponse::new(
                    RegistrationStatus::Accepted,
                    self.0,
                    300,
                )),
                CsRequest::MeterValues(reading) => {
                    ctx.session::<Inbox>()
                        .ok_or_else(|| CallError::internal("missing station inbox"))?
                        .try_send(reading)
                        .map_err(|_| CallError::internal("station inbox unavailable"))?;
                    ctx.reply(&MeterValuesResponse::new())
                }
                other => Err(CallError::not_supported(other.action().as_str())),
            }
        })
    }
}

type AppliedLimit = Arc<OnceLock<Decimal>>;

struct SimulatedCharger {
    issued_at: DateTime,
    applied_watts: AppliedLimit,
}

impl Handler for SimulatedCharger {
    fn on_request(
        &self,
        ctx: Ctx,
        req: IncomingRequest,
    ) -> BoxFuture<'_, Result<Box<RawValue>, CallError>> {
        Box::pin(async move {
            match ctx.decode::<CsmsRequest>(&req)? {
                CsmsRequest::SetChargingProfile(request) => {
                    let profile = request.cs_charging_profiles;
                    let schedule = profile.charging_schedule;
                    let period = schedule.charging_schedule_period.first();
                    let status = match period {
                        Some(period)
                            if request.connector_id == 0
                                && profile.charging_profile_purpose
                                    == ChargingProfilePurpose::ChargePointMaxProfile
                                && profile.charging_profile_kind
                                    == ChargingProfileKind::Absolute
                                && schedule.start_schedule == Some(self.issued_at)
                                && schedule.charging_rate_unit == ChargingRateUnit::W
                                && schedule.charging_schedule_period.len() == 1
                                && period.start_period == 0
                                && period.limit >= Decimal::ZERO
                                && self.applied_watts.set(period.limit).is_ok() =>
                        {
                            ChargingProfileStatus::Accepted
                        }
                        _ => ChargingProfileStatus::Rejected,
                    };
                    ctx.reply(&SetChargingProfileResponse::new(status))
                }
                other => Err(CallError::not_supported(other.action().as_str())),
            }
        })
    }
}

fn station_profile(at: DateTime, watts: i32) -> SetChargingProfileRequest {
    let schedule = ChargingSchedule::new(
        ChargingRateUnit::W,
        vec![ChargingSchedulePeriod::new(0, watts)],
    )
    .with_start_schedule(at);
    let profile = ChargingProfile::new(
        1,
        0,
        ChargingProfilePurpose::ChargePointMaxProfile,
        ChargingProfileKind::Absolute,
        schedule,
    );
    SetChargingProfileRequest::new(0, profile)
}

pub(super) async fn run(script: Script) -> Result<Recording> {
    let profile = station_profile(
        script.timeline.requested_at,
        super::BANK_LIMIT_KW * 1000 / 2,
    );
    record_bank(script, profile).await
}

async fn record_bank(script: Script, profile: SetChargingProfileRequest) -> Result<Recording> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let keys = [
        BasicAuthPassword::generate(Version::V1_6)?,
        BasicAuthPassword::generate(Version::V1_6)?,
    ];
    let expected = keys.clone();
    let (west1, rx1) = mpsc::channel(1);
    let (west2, rx2) = mpsc::channel(1);
    let csms = Csms::builder()
        .bind(addr)
        .versions([Version::V1_6])
        .max_connections(2)
        .supersede_existing(false)
        .authenticate(move |auth: Auth| {
            let provisioned = match auth.identity.as_str() {
                "West-01" => Some((&expected[0], &west1)),
                "West-02" => Some((&expected[1], &west2)),
                _ => None,
            };
            let verdict = match (provisioned, &auth.credentials) {
                (Some((key, inbox)), Credentials::Basic { password, .. })
                    if key.verify(password) =>
                {
                    Admission::new().context(inbox.clone()).into()
                }
                _ => AuthOutcome::Reject,
            };
            std::future::ready(verdict)
        })
        .handler(CentralSystem(script.timeline.requested_at))
        .build()?;
    let central = csms.handle();
    let url = format!("ws://{addr}/ocpp");
    let mut peers = Vec::with_capacity(2);
    let capture = async {
        let mut start = |id, key| -> Result<(Handle, AppliedLimit)> {
            let limit = AppliedLimit::default();
            let session = ChargePoint::builder()
                .identity(id)?
                .url(&url)
                .versions([Version::V1_6])
                .password(key)
                .reconnect(false)
                .handler(SimulatedCharger {
                    issued_at: script.timeline.requested_at,
                    applied_watts: limit.clone(),
                })
                .build()?
                .spawn()?;
            peers.push(session.clone());
            Ok((session, limit))
        };
        let [key1, key2] = keys;
        let west1 = start("West-01", key1)?;
        let west2 = start("West-02", key2)?;
        let (a, b) = tokio::try_join!(
            record(&central, west1, rx1, script.west_01, profile.clone()),
            record(&central, west2, rx2, script.west_02, profile),
        )?;
        Ok(Recording {
            timeline: script.timeline,
            traces: [a, b],
        })
    };
    let result = tokio::select! {
        result = csms.serve_on(listener) => Err(anyhow::anyhow!("local OCPP listener ended: {result:?}")),
        result = timeout(Duration::from_secs(10), capture) => result.context("timed out recording the local OCPP stations").and_then(|recording| recording),
    };
    let drain = Duration::from_millis(250);
    timeout(Duration::from_secs(1), async {
        for peer in peers {
            peer.shutdown(drain).await;
        }
        central.shutdown(drain).await;
    })
    .await
    .context("timed out closing local OCPP sessions")?;
    result
}

async fn record(
    central: &CsmsHandle,
    (station, applied): (Handle, AppliedLimit),
    mut inbox: mpsc::Receiver<MeterValuesRequest>,
    script: StationScript,
    profile: SetChargingProfileRequest,
) -> Result<Trace> {
    let boot = station
        .call(BootNotificationRequest::new("EMS", "Virtual wallbox"))
        .await
        .with_context(|| format!("{} BootNotification failed", station.identity()))?;
    ensure!(
        boot.status == RegistrationStatus::Accepted,
        "station registration refused"
    );
    station.wait_ready().await?;
    let response = central
        .call(station.identity(), profile)
        .await
        .with_context(|| format!("{} SetChargingProfile failed", station.identity()))?;
    let profile_response = Received {
        received_at: script.profile_reply_at,
        payload: response,
    };
    // The first sample is taken during ramp-down, before the accepted limit takes effect.
    let initial = exchange(&station, &mut inbox, script.initial).await?;
    let west2 = station.identity().as_str() == "West-02";
    let mut finals = BTreeMap::new();
    // Independent continuations from the first observation. A missing station sends no CALL.
    for outcome in [
        Outcome::Confirmed,
        Outcome::Above,
        Outcome::Stale,
        Outcome::Delayed,
        Outcome::Missing,
        Outcome::Zero,
    ] {
        let mut event = match outcome {
            Outcome::Missing if west2 => continue,
            Outcome::Stale if west2 => script.stale.clone(),
            _ => script.demand_at_deadline.clone(),
        };
        if outcome == Outcome::Delayed
            && let Some(at) = script.delayed_receipt_at
        {
            event.received_at = at;
        }
        if !(outcome == Outcome::Stale && west2) {
            // The fault branch tracks a kilowatt above its setpoint. An idle EV draws zero.
            let ceiling = match (outcome, applied.get().copied()) {
                (Outcome::Zero, _) if west2 => Some(Decimal::ZERO),
                (Outcome::Above, Some(limit)) => Some(
                    limit
                        .checked_add(1000.into())
                        .context("setpoint overflow")?,
                ),
                (_, limit) => limit,
            };
            if let Some(watts) = ceiling {
                meter_demand(&mut event.payload, watts)?;
            }
        }
        finals.insert(outcome, exchange(&station, &mut inbox, event).await?);
    }
    Ok(Trace {
        profile_response,
        initial,
        finals,
    })
}

fn meter_demand(readings: &mut [MeterValuesRequest], watts: Decimal) -> Result<()> {
    for sample in readings
        .iter_mut()
        .flat_map(|r| &mut r.meter_value)
        .flat_map(|r| &mut r.sampled_value)
    {
        if sample.measurand != Some(Measurand::PowerActiveImport) {
            continue;
        }
        let ceiling = match sample.unit {
            Some(UnitOfMeasure::W) => watts,
            Some(UnitOfMeasure::KW) => watts
                .checked_pow10(-3)
                .context("kilowatt conversion overflow")?,
            _ => continue,
        };
        let demand = sample
            .value
            .parse::<Decimal>()
            .context("invalid simulated EV demand")?;
        sample.value = demand.min(ceiling).to_string();
    }
    Ok(())
}

async fn exchange(
    station: &Handle,
    inbox: &mut mpsc::Receiver<MeterValuesRequest>,
    event: Received<Vec<MeterValuesRequest>>,
) -> Result<Received<Vec<MeterValuesRequest>>> {
    let mut readings = Vec::with_capacity(event.payload.len());
    for request in event.payload {
        station
            .call(request)
            .await
            .with_context(|| format!("{} MeterValues failed", station.identity()))?;
        readings.push(inbox.recv().await.context("station inbox closed")?);
    }
    Ok(Received {
        received_at: event.received_at,
        payload: readings,
    })
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Script, record_bank, station_profile};

    fn fixture() -> Script {
        serde_json::from_str(include_str!("../../examples/charging-bank.json")).unwrap()
    }

    #[tokio::test]
    async fn changing_the_wire_profile_changes_metered_import_after_ramp_down() {
        let script = fixture();
        let profile = station_profile(script.timeline.requested_at, 8_500);
        let recording = record_bank(script, profile).await.unwrap();
        let observed = recording.stations(Outcome::Confirmed, recording.timeline.deadline_at);
        for station in observed {
            assert_eq!(station.profile_accepted, Some(true));
            assert_eq!(station.initial_kw, Some(14.0));
            assert_eq!(station.final_kw, Some(8.5));
        }
    }

    #[tokio::test]
    async fn rejected_connector_profile_leaves_measured_demand_unconstrained() {
        let script = fixture();
        let mut profile = station_profile(script.timeline.requested_at, 8_500);
        profile.connector_id = 1;
        let recording = record_bank(script, profile).await.unwrap();
        for station in recording.stations(Outcome::Confirmed, recording.timeline.deadline_at) {
            assert_eq!(station.profile_accepted, Some(false));
            assert_eq!(station.final_kw, Some(14.0));
        }
    }
}
