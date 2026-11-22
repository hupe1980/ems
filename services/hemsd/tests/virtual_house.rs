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
async fn pair(scenario: Scenario, at: Duration) -> (House, hemsd::Running, ShutdownTrigger) {
    let (ship, api) = (free_port(), free_port());
    let house = house::start(house::Options {
        scenario: scenario.clone(),
        listen: "127.0.0.1:0".parse().expect("an address"),
        household: Some(ship),
        at: Some(at),
        identity: None,
        exposed: false,
    })
    .await
    .expect("the house starts");

    let written = house::configuration(&house, &scenario, Some(ship), api).expect("a config");
    let settings: hemsd::Settings = toml::from_str(&written).expect("the box reads what it wrote");
    let (signal, trigger) = Shutdown::channel();
    let running = hemsd::runtime::run(&settings, &Health::new(), &signal)
        .await
        .expect("the box starts on the house's own configuration");
    (house, running, trigger)
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

#[tokio::test(flavor = "multi_thread")]
async fn every_device_is_discovered_read_and_obeyed_over_modbus() {
    // A cloudless June noon: the roof offers well over what the household may
    // feed in, so a reading and a curtailment both mean something.
    let mut scenario = Scenario::summer_surplus(HouseholdConfig::default());
    scenario.cloudiness = 0.0;
    let rating = scenario.config.pv.expect("the reference roof").ac_nominal;
    let (house, running, trigger) = pair(scenario, Duration::hours(12)).await;
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
    let ceiling = until(
        StdDuration::from_secs(30),
        "the box's ceiling is in force on the inverter",
        async || {
            let commanded = running.status.lock().await.commanded.get(&pv)?.outflow();
            let fraction = house.curtailment()?;
            ((fraction * rating.get() - commanded.get()).abs() <= 0.005 * rating.get() + 1.0)
                .then_some(rating * fraction)
        },
    )
    .await;
    assert!(ceiling.get() > 0.0, "limited, not disconnected");
    until(
        StdDuration::from_secs(30),
        "the roof follows the ceiling and the box reads it",
        async || {
            let reading = house.reading();
            let state = running
                .registry
                .lock()
                .await
                .state(None, OffsetDateTime::now_utc());
            let believed = state.assets.get(&pv)?.power?.outflow().get();
            (reading.available_w > ceiling.get() * 1.05
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
    let (house, running, trigger) =
        pair(scenario, Duration::hours(17) - Duration::seconds(20)).await;

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

    trigger.trigger();
}
