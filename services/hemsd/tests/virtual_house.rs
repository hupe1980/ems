//! The box against a house it did not write.
//!
//! `hemsd house` on one side — SunSpec devices behind `tokio-modbus`'s server, a
//! Steuerbox on `eebus`'s Energy Guard — and an ordinary `hemsd run` on the
//! other, started from the configuration the house wrote for it. Everything
//! between them is a socket.
//!
//! Real time, and behind the `house` feature: the failsafe test waits out
//! `[LPC-911]`'s two minutes of silence, which no clock here can shorten.
#![cfg(feature = "house")]

use std::net::SocketAddr;
use std::time::Duration as StdDuration;

use ems_service::shutdown::ShutdownTrigger;
use ems_service::{Health, Shutdown};
use hemsd::house::{self, House};
use hemsd::{HouseholdConfig, Scenario};
use time::{Duration, OffsetDateTime};

/// A port nothing is listening on, for the box to bind a moment later.
fn free_port() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a free loopback port")
}

/// A house at `at` of `scenario`'s day, and the box that manages it.
async fn pair(
    scenario: Scenario,
    at: Duration,
    with_tank: bool,
    with_room: bool,
    with_ev: bool,
) -> (House, hemsd::Running, Health, ShutdownTrigger) {
    let (ship, api) = (free_port(), free_port());
    let house = house::start(house::Options {
        scenario: scenario.clone(),
        listen: "127.0.0.1:0".parse().expect("an address"),
        household: Some(ship),
        at: Some(at),
        identity: None,
        exposed: false,
        appliances: house::Appliances {
            tank: with_tank,
            room: with_room,
            ev: with_ev,
        },
    })
    .await
    .expect("the house starts");

    let written =
        house::configuration(&house, &scenario, Some(ship), api, false).expect("a config");
    let settings: hemsd::Settings = toml::from_str(&written).expect("the box reads what it wrote");
    let (signal, trigger) = Shutdown::channel();
    let health = Health::new();
    let running = hemsd::runtime::run(&settings, &health, &signal)
        .await
        .expect("the box starts on the house's own configuration");
    (house, running, health, trigger)
}

/// Poll `check` every 200 ms until it answers or `within` passes.
async fn until<T>(within: StdDuration, what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(StdDuration::from_millis(200)).await;
    }
}

async fn check_heat_readback(
    house: &House,
    running: &hemsd::Running,
    id: &ems_core::prelude::AssetId,
    rated_w: f64,
    band: std::ops::RangeInclusive<f64>,
) {
    use ems_core::prelude::UserOverride;
    let meter = &running.household.grid_meter;
    let mut readings = Vec::new();
    for (action, expected) in [
        (UserOverride::Pause, 0.0),
        (UserOverride::Boost, rated_w),
        (UserOverride::Pause, 0.0),
    ] {
        running
            .overrides
            .set(id.clone(), action, None, OffsetDateTime::now_utc())
            .await;
        let watts = until(
            StdDuration::from_secs(30),
            "appliance and connection readback",
            async || {
                let state = running
                    .registry
                    .lock()
                    .await
                    .state(Some(meter), OffsetDateTime::now_utc());
                let device = state.assets.get(id)?;
                let temp = device.temperature_c?;
                let grid = state.grid?.power?.get();
                (band.contains(&temp)
                    && (device.power?.get() - expected).abs() < 1.0
                    && (house.reading().grid_w - grid).abs() < 20.0)
                    .then_some(grid)
            },
        )
        .await;
        readings.push(watts);
    }
    assert!(
        (readings[1] - readings[0] - rated_w).abs() < 30.0,
        "{readings:?}"
    );
    assert!((readings[2] - readings[0]).abs() < 30.0, "{readings:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gateway_switches_the_tank_and_measures_the_load_at_the_connection() {
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig {
        battery: None,
        ..Default::default()
    });
    let tank = scenario.config.dhw.unwrap();
    let (house, running, _, stop) = pair(scenario, Duration::hours(3), true, false, false).await;
    let id = running.household.dhw.clone().unwrap();
    check_heat_readback(
        &house,
        &running,
        &id,
        tank.heater.get(),
        tank.t_min_c..=tank.t_max_c,
    )
    .await;
    stop.trigger();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_commissioned_wallbox_reports_outlet_energy_and_responds_to_the_gateway() {
    use ems_core::prelude::UserOverride;
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig {
        battery: None,
        ..Default::default()
    });
    let (house, running, _, stop) = pair(scenario, Duration::hours(3), false, false, true).await;
    let port = running.household.evse.as_ref().unwrap();
    let grid = &running.household.grid_meter;
    let mut readings = Vec::new();
    for (action, charging) in [
        (UserOverride::Pause, false),
        (UserOverride::Boost, true),
        (UserOverride::Pause, false),
    ] {
        running
            .overrides
            .set(port.clone(), action, None, OffsetDateTime::now_utc())
            .await;
        let watts = until(
            StdDuration::from_secs(40),
            "metered EV response",
            async || {
                let now = OffsetDateTime::now_utc();
                let state = running.registry.lock().await.observe(Some(grid), now);
                let car = state.vehicles.get(port)?;
                let device = state.state.assets.get(port)?;
                let flow = device.power?.get();
                let feeder = state.state.grid?.power?.get();
                (car.current_soc(now).is_some()
                    && car.capacity_wh == Some(60_000.0)
                    && (house.reading().grid_w - feeder).abs() < 20.0
                    && if charging {
                        flow >= 4_140.0 && device.energy_in?.get() > 0.0
                    } else {
                        flow == 0.0
                    })
                .then_some((flow, feeder))
            },
        )
        .await;
        readings.push(watts);
    }
    assert!((readings[1].1 - readings[0].1 - readings[1].0).abs() < 30.0);
    assert!((readings[2].1 - readings[0].1).abs() < 30.0);
    until(
        StdDuration::from_secs(60),
        "planner discovers the measured EV session",
        async || {
            let car = running.car.lock().await;
            car.as_ref()
                .filter(|session| {
                    (session.capacity.kwh() - 60.0).abs() < 0.001
                        && (session.energy_now.kwh() - 18.0).abs() < 0.1
                })
                .map(|_| ())
        },
    )
    .await;
    stop.trigger();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gateway_measures_room_heat_at_the_connection() {
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig {
        battery: None,
        ..Default::default()
    });
    let pump = scenario.config.heat_pump.unwrap();
    let (house, running, _, stop) = pair(scenario, Duration::hours(3), false, true, false).await;
    let id = running.household.heat_pump.clone().unwrap();
    check_heat_readback(
        &house,
        &running,
        &id,
        pump.power.get(),
        pump.comfort_min_c..=pump.comfort_max_c,
    )
    .await;
    stop.trigger();
}

#[tokio::test(flavor = "multi_thread")]
async fn grid_curtailment_limits_a_boosted_room() {
    use ems_core::prelude::{Power, UserOverride};
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig {
        evse: None,
        battery: None,
        ..Default::default()
    });
    use tokio_modbus::prelude::{Reader, Slave};
    let limit = scenario.grid_event.unwrap().2;
    let (house, running, _, stop) = pair(scenario, Duration::hours(17), false, true, false).await;
    let mut peer = tokio_modbus::client::tcp::connect_slave(house.modbus, Slave(5))
        .await
        .unwrap();
    let id = running.household.heat_pump.clone().unwrap();
    running
        .overrides
        .set(
            id.clone(),
            UserOverride::Boost,
            None,
            OffsetDateTime::now_utc(),
        )
        .await;
    until(
        StdDuration::from_secs(60),
        "acknowledged grid limit and measured heat pump reduction",
        async || {
            let words = peer.read_holding_registers(1, 2).await.ok()?.ok()?;
            let commanded = *running.status.lock().await.commanded.get(&id)?;
            let registry = running.registry.lock().await;
            let state = registry.state(None, OffsetDateTime::now_utc());
            (registry.limits().steuve_ceiling == Some(limit)
                && commanded > Power::ZERO
                && commanded <= limit
                && state.assets.get(&id)?.power? == Power::ZERO
                && words == [0, 1])
            .then_some(())
        },
    )
    .await;
    stop.trigger();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shadow_gateway_reads_the_meter_and_never_writes_a_setpoint() {
    use ems_core::prelude::{Command, Power, Reason, Setpoint, UserOverride};
    let scenario = Scenario::summer_surplus(HouseholdConfig::default());
    let house = house::start(house::Options {
        scenario: scenario.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        household: None,
        at: Some(Duration::hours(12)),
        identity: None,
        exposed: false,
        appliances: Default::default(),
    })
    .await
    .unwrap();
    let config = house::configuration(&house, &scenario, None, free_port(), true).unwrap();
    let mut settings: hemsd::Settings = toml::from_str(&config).unwrap();
    let state = std::env::temp_dir().join(format!("hems-shadow-{}.redb", uuid::Uuid::new_v4()));
    settings.store_path = Some(state.clone());
    settings.mode = hemsd::config::OperatingMode::Shadow;
    let now = OffsetDateTime::now_utc();
    assert!(
        hemsd::runtime::assemble(&settings, None, now)
            .err()
            .unwrap()
            .to_string()
            .contains("external_limit_controller")
    );
    settings.external_limit_controller = Some("Independent site PLC".into());
    settings.ship.listen = Some(free_port().to_string());
    assert!(hemsd::runtime::assemble(&settings, None, now).is_err());
    settings.ship.listen = None;
    let (signal, trigger) = Shutdown::channel();
    let running = hemsd::runtime::run(&settings, &Health::new(), &signal)
        .await
        .unwrap();
    until(
        StdDuration::from_secs(30),
        "shadow meter telemetry",
        async || running.status.lock().await.grid,
    )
    .await;
    let pv = running.household.pv.clone().unwrap();
    for reason in [Reason::Cem, Reason::User(UserOverride::Boost)] {
        let attempted = Setpoint::new(
            pv.clone(),
            Command::ProductionCeiling(Power::ZERO),
            reason,
            now,
        )
        .unwrap();
        assert!(
            running
                .registry
                .lock()
                .await
                .command(&attempted, now)
                .is_err()
        );
    }
    running
        .overrides
        .set(pv, UserOverride::Boost, None, OffsetDateTime::now_utc())
        .await;
    let tick = running.status.lock().await.at;
    until(
        StdDuration::from_secs(10),
        "a subsequent shadow control tick",
        async || {
            let status = running.status.lock().await;
            (status.at > tick && status.shadow && status.commanded.is_empty()).then_some(())
        },
    )
    .await;
    assert_eq!(house.write_requests(), 0);
    trigger.trigger();
    drop(running);
    let _ = std::fs::remove_file(state);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_device_is_discovered_read_and_obeyed_over_modbus() {
    // A cloudless June noon: the roof offers well over what the household may
    // feed in, so a reading and a curtailment both mean something.
    let mut scenario = Scenario::summer_surplus(HouseholdConfig::default());
    scenario.cloudiness = 0.0;
    let rating = scenario.config.pv.expect("the reference roof").ac_nominal;
    let (house, running, _, trigger) =
        pair(scenario, Duration::hours(12), false, false, false).await;
    let pv = running.household.pv.clone().expect("the reference roof");
    let battery = running.household.battery.clone().expect("a battery");
    let meter = running.household.grid_meter.clone();

    // Discovery and reads: each device walked by its model chain, and what the
    // box believes is what the house is doing. Polled until they agree, because
    // the house moves every second and the box reads every second: two samples
    // are never of the same instant.
    until(
        StdDuration::from_secs(30),
        "the box believes what the house is doing",
        async || {
            let state = running
                .registry
                .lock()
                .await
                .state(Some(&meter), OffsetDateTime::now_utc());
            let reading = house.reading();
            let roof = state.assets.get(&pv)?.power?.outflow().get();
            let grid = state.grid?.power?.get();
            let soc = state.assets.get(&battery)?.soc?.fraction();
            (reading.pv_w > 100.0
                && (roof - reading.pv_w).abs() < 0.1 * reading.pv_w
                && (grid - reading.grid_w).abs() < 0.1 * reading.pv_w
                && (soc - reading.soc).abs() < 0.011)
                .then_some(())
        },
    )
    .await;
    // Every device is heard — the Steuerbox too, whose SHIP pairing runs beside
    // the Modbus reads and can finish after them on a slow machine.
    until(
        StdDuration::from_secs(60),
        "every device the house serves is heard",
        async || {
            running
                .registry
                .lock()
                .await
                .silent(OffsetDateTime::now_utc())
                .next()
                .is_none()
                .then_some(())
        },
    )
    .await;

    // A write is honoured. Nothing here commands the inverter — the box's own
    // control loop does, with the ceiling § 9 EEG puts on this roof — so what is
    // asserted is the running product's write: it arrives as model 123's
    // percentage at the register the model keeps it in, the simulated array
    // follows it, and the box's next read shows the result.
    // The controller can revise its ceiling between reads. Compare the active
    // register, the latest command and both power readings in the same poll.
    until(
        StdDuration::from_secs(30),
        "the roof follows the active ceiling and the box reads it",
        async || {
            let commanded = running.status.lock().await.commanded.get(&pv)?.outflow();
            let ceiling = rating * house.curtailment()?;
            let reading = house.reading();
            let state = running
                .registry
                .lock()
                .await
                .state(None, OffsetDateTime::now_utc());
            let believed = state.assets.get(&pv)?.power?.outflow().get();
            (ceiling.get() > 0.0
                && (ceiling.get() - commanded.get()).abs() <= 0.005 * rating.get() + 1.0
                && reading.available_w > ceiling.get() * 1.05
                && (reading.pv_w - ceiling.get()).abs() < 0.02 * ceiling.get()
                && (believed - ceiling.get()).abs() < 0.02 * ceiling.get())
            .then_some(())
        },
    )
    .await;
    let refused = running
        .registry
        .lock()
        .await
        .disobedient()
        .any(|(asset, _)| *asset == pv);
    assert!(!refused, "and the read-back confirmed it");

    trigger.trigger();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_steuerbox_reduces_the_household_and_its_silence_is_a_failsafe() {
    // Twenty seconds before the winter day's 17:00 reduction.
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
    let limit = scenario.grid_event.expect("a reduction").2;
    let (house, running, health, trigger) = pair(
        scenario,
        Duration::hours(17) - Duration::seconds(20),
        false,
        false,
        false,
    )
    .await;

    // Paired over SHIP, and nothing required yet.
    let mut seen = house.steuerbox.clone();
    tokio::time::timeout(StdDuration::from_secs(60), seen.wait_for(|s| s.connected))
        .await
        .expect("the Steuerbox reaches the box")
        .expect("the house is running");
    assert_eq!(seen.borrow().accepted_w, None, "no reduction before 17:00");

    // 17:00 — the limit is written, acknowledged, and is what the guard holds
    // the controllable devices to.
    tokio::time::timeout(
        StdDuration::from_secs(90),
        seen.wait_for(|s| s.accepted_w.is_some()),
    )
    .await
    .expect("the reduction is acknowledged — the § 14a evidence")
    .expect("the house is running");
    let accepted = seen.borrow().accepted_w.expect("accepted");
    assert!((accepted - limit.get()).abs() < 1.0, "{accepted} W");
    until(
        StdDuration::from_secs(30),
        "the guard holds the operator's limit",
        async || {
            let limits = running.registry.lock().await.limits();
            (limits.steuve_ceiling == Some(limit) && !limits.in_failsafe).then_some(())
        },
    )
    .await;

    // The control box falls silent. `[LPC-911]`: after 120 s without a
    // heartbeat the household restrains itself.
    assert!(health.readiness().probes["grid"].ready);
    house.silence_steuerbox();
    until(
        StdDuration::from_secs(200),
        "a failsafe on a silent Steuerbox",
        async || {
            running
                .registry
                .lock()
                .await
                .limits()
                .in_failsafe
                .then_some(())
        },
    )
    .await;

    let grid = &health.readiness().probes["grid"];
    assert!(!grid.ready && grid.last_good.is_some());
    trigger.trigger();
}
