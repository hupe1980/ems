//! Virtual outlet and two authenticated WebSocket sessions. The outlet retains its register.
use super::{CONNECTOR, MAX_W, Meter, STATION, TX, Times};
use anyhow::{Context, Result, ensure};
use ocpp_kit::{
    RawValue, Version,
    engine::IncomingRequest,
    rpc::CallError,
    transport::{BasicAuthPassword, BoxFuture, CsmsHandle, Ctx, Handle, Handler, Station},
    types::DateTime,
    v1_6::{
        BootNotificationRequest, ChargingProfile, ChargingProfileKind, ChargingProfilePurpose,
        ChargingProfileStatus, ChargingRateUnit, ChargingSchedule, ChargingSchedulePeriod,
        CsmsRequest, MeterValue, MeterValuesRequest, RegistrationStatus, SampledValue,
        SetChargingProfileRequest, SetChargingProfileResponse, StartTransactionRequest,
        StopTransactionRequest,
    },
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

struct Wallbox {
    at: DateTime,
    watts: Arc<OnceLock<i32>>,
}

impl Wallbox {
    fn apply(&self, request: &SetChargingProfileRequest) -> bool {
        let profile = &request.cs_charging_profiles;
        let schedule = &profile.charging_schedule;
        let [period] = schedule.charging_schedule_period.as_slice() else {
            return false;
        };
        if request.connector_id != CONNECTOR
            || profile.transaction_id != Some(TX)
            || profile.charging_profile_purpose != ChargingProfilePurpose::TxProfile
            || profile.charging_profile_kind != ChargingProfileKind::Absolute
            || schedule.duration != Some(900)
            || schedule.start_schedule != Some(self.at)
            || schedule.charging_rate_unit != ChargingRateUnit::W
            || period.start_period != 0
        {
            return false;
        }
        let Ok(whole_watts) = i64::try_from(period.limit) else {
            return false;
        };
        let Ok(watts) = i32::try_from(whole_watts) else {
            return false;
        };
        (0..=MAX_W).contains(&watts) && self.watts.set(watts).is_ok()
    }
}

impl Handler for Wallbox {
    fn on_request(
        &self,
        ctx: Ctx,
        req: IncomingRequest,
    ) -> BoxFuture<'_, Result<Box<RawValue>, CallError>> {
        Box::pin(async move {
            let request = ctx.decode::<CsmsRequest>(&req)?;
            let CsmsRequest::SetChargingProfile(request) = request else {
                return Err(CallError::not_supported(request.action().as_str()));
            };
            ctx.reply(&SetChargingProfileResponse::new(if self.apply(&request) {
                ChargingProfileStatus::Accepted
            } else {
                ChargingProfileStatus::Rejected
            }))
        })
    }
}

async fn connect(
    addr: SocketAddr,
    password: BasicAuthPassword,
    at: DateTime,
) -> Result<(Handle, Arc<OnceLock<i32>>)> {
    let watts = Arc::default();
    let session = Station::builder()
        .identity(STATION)?
        .url(format!("ws://{addr}/ocpp"))
        .versions([Version::V1_6])
        .password(password)
        .reconnect(false)
        .handler(Wallbox {
            at,
            watts: Arc::clone(&watts),
        })
        .build()?
        .spawn()?;
    let boot = session
        .call(BootNotificationRequest::new("EMS", "Virtual home outlet"))
        .await?;
    ensure!(
        boot.status == RegistrationStatus::Accepted,
        "registration refused"
    );
    session.wait_ready().await?;
    Ok((session, watts))
}

async fn quarter(
    central: &CsmsHandle,
    station: &Handle,
    applied: &OnceLock<i32>,
    at: DateTime,
    watts: i32,
) -> Result<i32> {
    let schedule = ChargingSchedule::new(
        ChargingRateUnit::W,
        vec![ChargingSchedulePeriod::new(0, watts)],
    )
    .with_start_schedule(at)
    .with_duration(900);
    let profile = ChargingProfile::new(
        1,
        0,
        ChargingProfilePurpose::TxProfile,
        ChargingProfileKind::Absolute,
        schedule,
    )
    .with_transaction_id(TX);
    let response = central
        .call(
            station.identity(),
            SetChargingProfileRequest::new(CONNECTOR, profile),
        )
        .await?;
    ensure!(
        response.status == ChargingProfileStatus::Accepted,
        "transaction profile refused"
    );
    // Both devices integrate the rate actually received by the outlet.
    applied
        .get()
        .copied()
        .context("wallbox has no active limit")
}

pub(super) async fn drive(
    central: &CsmsHandle,
    addr: SocketAddr,
    password: BasicAuthPassword,
    times: Times,
    goal: i32,
    meter: &Mutex<Meter>,
    missing_close: bool,
) -> Result<super::grid::Balance> {
    let at = time::OffsetDateTime::from_unix_timestamp(times.start.timestamp().as_second())?;
    let mut grid = super::grid::Grid::start(at).await?;
    let open = grid.sample().await?;
    let site = super::household()?;
    let (station, applied) = connect(addr, password.clone(), times.start).await?;
    let mut register = 48210;
    let start = station
        .call(StartTransactionRequest::new(
            CONNECTOR,
            "household",
            register,
            times.start,
        ))
        .await?;
    ensure!(start.transaction_id == TX, "unexpected transaction");
    // Spread the goal over three quarters, rounding up to whole outlet Wh.
    let initial_w = super::allowed_watts(&site, times.start, (goal + 2) / 3 * 4, open)?;
    let first = quarter(central, &station, &applied, times.start, initial_w).await?;
    register += first / 4;
    let pause = grid.quarter(first).await?;
    station.shutdown(Duration::from_millis(250)).await;
    while central.sessions().await.contains(station.identity()) {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let resume = grid.quarter(0).await?;
    let (returned, applied) = connect(addr, password, times.resume).await?;
    let sample = |at, wh: i32| {
        MeterValuesRequest::new(
            CONNECTOR,
            vec![MeterValue::new(at, vec![SampledValue::new(wh.to_string())])],
        )
        .with_transaction_id(TX)
    };
    // The first upload after reconnect is buffered. Its arrival cannot authorize a new limit.
    returned.call(sample(times.start, 48210)).await?;
    ensure!(
        meter.lock().expect("departure meter").resume.is_none(),
        "buffered reading counted as fresh"
    );
    let reading = sample(times.resume, register);
    returned.call(reading.clone()).await?;
    returned.call(reading).await?;
    let remaining = {
        let observed = meter.lock().expect("departure meter");
        let delivered = observed.resume.context("missing rejoin register")?
            - observed.start.context("missing starting register")?;
        (goal - delivered).max(0)
    };
    let last = quarter(
        central,
        &returned,
        &applied,
        times.resume,
        super::allowed_watts(&site, times.resume, remaining * 4, resume)?,
    )
    .await?;
    register += last / 4;
    let close = grid.quarter(last).await?;
    if !missing_close {
        returned
            .call(StopTransactionRequest::new(register, times.leave, TX))
            .await?;
    }
    returned.shutdown(Duration::from_millis(250)).await;
    super::grid::Balance::from_samples([&open, &pause, &resume, &close])
}
