//! A household on the wire, for a real `hemsd run` to manage.
//!
//! `hemsd demo` replays decisions; nothing in it crosses a socket. This is the
//! other half: one process that *is* a house — an inverter, a meter and a
//! battery answering SunSpec over Modbus TCP from `hems-sim`'s physics under a
//! reference day's weather, and a Steuerbox that dials the box over SHIP and
//! writes that day's § 14a reduction. The box reaches it through its ordinary
//! drivers and cannot tell.
//!
//! Real time only: the box reads the system clock, so the house does too, and
//! [`Options::at`] chooses where in the reference day it starts.

pub mod fleet;
mod modbus;
pub mod registers;
pub mod steuerbox;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hems_core::prelude::{GeoPoint, Power, Slot};
use hems_forecast::ArrayModel;
use hems_sim::{BatterySim, PvSim};
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;

use crate::forecasting::Weather;
use crate::scenario::Scenario;
use registers::Unit;

/// The asset the § 14a limit applies to, as the reference household names it.
const CONNECTION_POINT: &str = "netzanschluss";

/// The Modbus unit of the inverter.
pub const UNIT_INVERTER: u8 = 1;
/// The Modbus unit of the meter at the grid connection.
pub const UNIT_METER: u8 = 2;
/// The Modbus unit of the battery.
pub const UNIT_BATTERY: u8 = 3;

/// How to start a house.
#[derive(Debug, Clone)]
pub struct Options {
    /// The reference day it lives.
    pub scenario: Scenario,
    /// Where Modbus TCP listens. Loopback, unless [`Options::exposed`].
    pub listen: SocketAddr,
    /// The box's SHIP address, for the Steuerbox to dial; `None` runs no
    /// Steuerbox.
    pub household: Option<SocketAddr>,
    /// Where in the day the house starts, since local midnight; `None` is the
    /// wall clock's own time of day.
    pub at: Option<Duration>,
    /// Where the Steuerbox keeps its certificate; `None` issues a fresh one.
    pub identity: Option<PathBuf>,
    /// Allow a listen address that is not loopback.
    pub exposed: bool,
}

/// The house's clock: the wall clock, moved onto the reference day.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    midnight: OffsetDateTime,
    offset: Duration,
    started: OffsetDateTime,
}

impl Clock {
    fn new(scenario: &Scenario, at: Option<Duration>, now: OffsetDateTime) -> Self {
        let offset = at.unwrap_or_else(|| {
            Duration::minutes(i64::from(Slot::containing(now).local_minute_of_day()))
        });
        Self {
            midnight: scenario.start(),
            offset,
            started: now,
        }
    }

    /// How far into the reference day the house is.
    #[must_use]
    pub fn since_midnight(&self) -> Duration {
        self.offset + (OffsetDateTime::now_utc() - self.started)
    }

    /// The instant of the reference day the house is living.
    #[must_use]
    pub fn now(&self) -> OffsetDateTime {
        self.midnight + self.since_midnight()
    }

    /// The instant of the reference day the house lives at wall-clock `real`.
    #[must_use]
    pub fn scenario_at(&self, real: OffsetDateTime) -> OffsetDateTime {
        self.midnight + self.offset + (real - self.started)
    }
}

/// What the house is doing, in watts, load convention aside: each a magnitude
/// with its own direction named.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Reading {
    /// What the roof could deliver.
    pub available_w: f64,
    /// What the inverter feeds in.
    pub pv_w: f64,
    /// What the household draws.
    pub load_w: f64,
    /// What the battery takes (positive) or gives (negative).
    pub battery_w: f64,
    /// What crosses the connection point, import positive.
    pub grid_w: f64,
    /// The battery's state of charge, 0–1.
    pub soc: f64,
}

/// The devices and the registers they publish.
#[derive(Debug)]
pub struct Physics {
    inverter: Unit,
    meter: Unit,
    battery: Unit,
    pv: PvSim,
    pack: BatterySim,
    array: ArrayModel,
    rating: Power,
    location: GeoPoint,
    weather: Weather,
    produced_wh: f64,
    imported_wh: f64,
    exported_wh: f64,
    /// The last step.
    pub reading: Reading,
}

impl Physics {
    fn new(scenario: &Scenario) -> anyhow::Result<Self> {
        let config = &scenario.config;
        let (Some(pv), Some(battery)) = (config.pv, config.battery) else {
            anyhow::bail!("the virtual house needs a roof and a battery in its configuration");
        };
        let mut pack = BatterySim::new(battery.kwh, battery.power);
        pack.stored = battery.kwh * battery.reserve_soc.fraction().max(0.30);
        Ok(Self {
            inverter: Unit::inverter(),
            meter: Unit::meter(),
            battery: Unit::battery(),
            pv: PvSim::default(),
            pack,
            array: ArrayModel::new(pv.kwp, pv.ac_nominal, pv.tilt_deg, pv.azimuth_deg),
            rating: pv.ac_nominal,
            location: config.location,
            weather: Weather::new(scenario.weather, scenario.cloudiness, scenario.outdoor_c),
            produced_wh: 0.0,
            imported_wh: 0.0,
            exported_wh: 0.0,
            reading: Reading::default(),
        })
    }

    /// Move every device on by `dt` at instant `at` of the reference day, and
    /// refresh the registers.
    ///
    /// The battery runs itself — it covers the house from the roof first, as an
    /// uncommanded pack does — because the box's SunSpec driver carries no
    /// storage setpoint to command it with.
    pub fn step(&mut self, at: OffsetDateTime, dt: Duration) {
        let hours = dt.as_seconds_f64() / 3600.0;
        let available = self.weather.production_at(&self.array, self.location, at);
        let ceiling = self
            .inverter
            .curtailment()
            .map_or(self.rating, |fraction| self.rating * fraction);
        let (pv, _) = self.pv.step(available, ceiling, dt);
        let produced = pv.outflow();
        let load = self
            .weather
            .load_at(at, crate::scenario::household_load(Slot::containing(at)));
        let battery = self.pack.step(produced - load, dt);
        let grid = load + battery - produced;

        self.produced_wh += produced.get() * hours;
        self.imported_wh += grid.get().max(0.0) * hours;
        self.exported_wh += (-grid.get()).max(0.0) * hours;
        self.reading = Reading {
            available_w: available.get(),
            pv_w: produced.get(),
            load_w: load.get(),
            battery_w: battery.get(),
            grid_w: grid.get(),
            soc: self.pack.soc().fraction(),
        };
        self.inverter.set_inverter(produced.get(), self.produced_wh);
        self.meter
            .set_meter(grid.get(), self.imported_wh, self.exported_wh);
        self.battery.set_battery(self.reading.soc);
    }
}

/// A running house.
#[derive(Debug)]
pub struct House {
    /// Where Modbus TCP is listening.
    pub modbus: SocketAddr,
    /// Where the house answers for its day's prices and sky, as `tariffd` and
    /// `forecastd` would.
    pub fleet: SocketAddr,
    /// The Steuerbox's SKI, for the box to trust; `None` without a Steuerbox.
    pub steuerbox_ski: Option<String>,
    /// What the Steuerbox has seen of the household.
    pub steuerbox: watch::Receiver<steuerbox::Seen>,
    /// The house's clock.
    pub clock: Clock,
    physics: Arc<Mutex<Physics>>,
    silenced: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl House {
    /// What the devices are doing now.
    #[must_use]
    pub fn reading(&self) -> Reading {
        self.physics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reading
    }

    /// The production ceiling the box has written to the inverter, as a
    /// fraction of its rating; `None` while no limit is in force.
    #[must_use]
    pub fn curtailment(&self) -> Option<f64> {
        self.physics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inverter
            .curtailment()
    }

    /// Stop the Steuerbox dead — no limit, no heartbeat — as a failed control
    /// box or a cut line would.
    pub fn silence_steuerbox(&self) {
        self.silenced.store(true, Ordering::Relaxed);
    }
}

impl Drop for House {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Start the house: bind Modbus, step the physics every second, and — where a
/// box address is given — start the Steuerbox.
///
/// # Errors
/// Where the address is not loopback and [`Options::exposed`] is unset, the
/// port cannot be bound, or the scenario has no roof or battery.
pub async fn start(options: Options) -> anyhow::Result<House> {
    if !options.listen.ip().is_loopback() && !options.exposed {
        anyhow::bail!(
            "the virtual house answers anyone who asks and has no credentials: \
             {} is not a loopback address (pass --exposed to mean it)",
            options.listen
        );
    }
    let now = OffsetDateTime::now_utc();
    let clock = Clock::new(&options.scenario, options.at, now);
    let mut physics = Physics::new(&options.scenario)?;
    physics.step(clock.now(), Duration::ZERO);
    let physics = Arc::new(Mutex::new(physics));

    let listener = tokio::net::TcpListener::bind(options.listen).await?;
    let modbus = listener.local_addr()?;
    let mut tasks = vec![tokio::spawn(modbus::serve(listener, Arc::clone(&physics)))];
    // The day's prices and sky, where `tariffd` and `forecastd` would answer
    // them, on the Modbus address's host and a port of its own.
    let (fleet, prices_and_sky) =
        fleet::serve(SocketAddr::new(modbus.ip(), 0), &options.scenario, clock).await?;
    tasks.push(prices_and_sky);

    let stepped = Arc::clone(&physics);
    tasks.push(tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_secs(1));
        every.tick().await;
        loop {
            every.tick().await;
            stepped
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .step(clock.now(), Duration::SECOND);
        }
    }));

    let (seen, steuerbox) = watch::channel(steuerbox::Seen::default());
    let silenced = Arc::new(AtomicBool::new(false));
    let mut steuerbox_ski = None;
    if let Some(household) = options.household {
        let identity = steuerbox::identity(options.identity.as_deref())?;
        steuerbox_ski = Some(identity.ski.to_string());
        let reduction = options
            .scenario
            .grid_event
            .map(|(from, until, limit)| (from, until, limit.get()));
        tasks.push(tokio::spawn(steuerbox::run(
            identity,
            household,
            reduction,
            clock,
            seen,
            Arc::clone(&silenced),
        )));
    }

    Ok(House {
        modbus,
        fleet,
        steuerbox_ski,
        steuerbox,
        clock,
        physics,
        silenced,
        tasks,
    })
}

/// The configuration of a box that manages `house`: the reference household,
/// its three SunSpec devices by unit, and the Steuerbox trusted by SKI.
///
/// # Errors
/// Where the scenario's household cannot be built.
pub fn configuration(
    house: &House,
    scenario: &Scenario,
    ship: Option<SocketAddr>,
    api: SocketAddr,
) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    let household =
        crate::Household::build(&scenario.config).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rating_kw = scenario.config.pv.map_or(0.0, |pv| pv.ac_nominal.kw());
    let mut out = format!(
        "# A box that manages the virtual house on {modbus}.\n\
         # Written by `hemsd house`; start it with `hemsd run --config <this file>`.\n\n\
         [service]\nlisten = \"{api}\"\n\n\
         [fleet]\ntariffd_url = \"http://{fleet}\"\nforecastd_url = \"http://{fleet}\"\n\
         location = \"{location}\"\n",
        modbus = house.modbus,
        fleet = house.fleet,
        location = fleet::LOCATION,
    );
    if let (Some(ship), Some(ski)) = (ship, &house.steuerbox_ski) {
        write!(
            out,
            "\n[ship]\nlisten = \"{ship}\"\ntrust = [\"{ski}\"]\nannounce = false\n"
        )?;
    }
    let mut device = |asset: &str, unit: u8, extra: &str| {
        write!(
            out,
            "\n[[drivers]]\nkind = \"sunspec\"\nasset = \"{asset}\"\n\
             address = \"{}\"\nunit = {unit}\n{extra}",
            house.modbus
        )
    };
    if let Some(pv) = &household.pv {
        device(
            pv.as_str(),
            UNIT_INVERTER,
            &format!("rating_kw = {rating_kw}\n"),
        )?;
    }
    device(
        household.grid_meter.as_str(),
        UNIT_METER,
        "listens_only = true\n",
    )?;
    if let Some(battery) = &household.battery {
        device(battery.as_str(), UNIT_BATTERY, "")?;
    }
    if ship.is_some() {
        write!(
            out,
            "\n[[drivers]]\nkind = \"eebus-lpc\"\nasset = \"{CONNECTION_POINT}\"\n"
        )?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HouseholdConfig;

    fn options(listen: &str) -> Options {
        Options {
            scenario: Scenario::winter_with_grid_event(HouseholdConfig::default()),
            listen: listen.parse().expect("an address"),
            household: None,
            at: Some(Duration::hours(12)),
            identity: None,
            exposed: false,
        }
    }

    /// The box that manages the house is told the house's own day, so it
    /// plans: the price of the slot the house lives in is the reference day's.
    #[tokio::test]
    async fn the_house_prices_the_slot_it_lives_in_from_its_own_day() {
        let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
        let house = start(options("127.0.0.1:0")).await.expect("a house");
        let now = Slot::containing(OffsetDateTime::now_utc()).start();
        let from = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
            .replace('+', "%2B");
        let prices: serde_json::Value = reqwest::get(format!(
            "http://{}/v1/prices?from={from}&slots=2",
            house.fleet
        ))
        .await
        .expect("an answer")
        .json()
        .await
        .expect("tariffd's shape");
        // `options` puts the house at 12:00: the forty-ninth slot of its day.
        assert_eq!(
            prices["points"][0]["price_ct"],
            scenario.prices_ct[48].to_string()
        );
        assert_eq!(prices["points"].as_array().map(Vec::len), Some(2));

        let sky: serde_json::Value = reqwest::get(format!(
            "http://{}/v1/weather/{}",
            house.fleet,
            fleet::LOCATION
        ))
        .await
        .expect("an answer")
        .json()
        .await
        .expect("forecastd's shape");
        assert_eq!(sky["points"].as_array().map(Vec::len), Some(192));

        let written = configuration(&house, &scenario, None, "127.0.0.1:8080".parse().unwrap())
            .expect("a configuration");
        assert!(written.contains(&format!("tariffd_url = \"http://{}\"", house.fleet)));
    }

    #[tokio::test]
    async fn a_house_is_not_served_to_the_network_by_accident() {
        let refused = start(options("0.0.0.0:0")).await.expect_err("not loopback");
        assert!(refused.to_string().contains("--exposed"), "{refused}");
        start(options("127.0.0.1:0")).await.expect("loopback");
    }

    #[tokio::test]
    async fn the_configuration_it_writes_is_one_the_box_accepts() {
        let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
        let house = start(Options {
            household: Some("127.0.0.1:4712".parse().expect("an address")),
            ..options("127.0.0.1:0")
        })
        .await
        .expect("a house");
        let written = configuration(
            &house,
            &scenario,
            Some("127.0.0.1:4712".parse().expect("an address")),
            "127.0.0.1:8080".parse().expect("an address"),
        )
        .expect("a configuration");
        let settings: crate::Settings = toml::from_str(&written).expect("it parses");
        assert_eq!(
            settings.drivers.len(),
            4,
            "three devices and the § 14a driver"
        );
        assert!(!settings.ship.announce, "a simulator's box stays off mDNS");
        crate::runtime::assemble(&settings, None, OffsetDateTime::now_utc())
            .expect("and describes a site the box will start on");
    }

    #[test]
    fn the_battery_covers_the_house_before_the_grid_does() {
        let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
        let mut physics = Physics::new(&scenario).expect("the reference household");
        // 19:00 in January: no sun, the evening peak, a pack with charge in it.
        let evening = scenario.start() + Duration::hours(19);
        for _ in 0..5 {
            physics.step(evening, Duration::SECOND);
        }
        let r = physics.reading;
        assert!(r.pv_w < 1.0 && r.load_w > 500.0, "{r:?}");
        assert!(r.battery_w < -400.0, "the pack discharges: {r:?}");
        assert!(
            r.grid_w.abs() < 50.0,
            "so the meter reads next to nothing: {r:?}"
        );
        assert!(
            (r.load_w + r.battery_w - r.pv_w - r.grid_w).abs() < 1e-6,
            "{r:?}"
        );
    }
}
