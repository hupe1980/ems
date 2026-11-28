//! Loopback household equipment for exercising the gateway through its ordinary drivers.
//!
//! The continuous house follows wall time from the selected reference-day offset.
//! SunSpec devices serve Modbus TCP, and the Steuerbox sends limits over SHIP.
//! Optional room, tank and wallbox peers share the connection meter. The separate
//! accelerated trials advance the plant clock to compare closed intervals.

mod appliance_map;
pub mod departure;
mod ev;
pub mod fleet;
mod grid;
mod modbus;
#[cfg(test)]
mod reference_trials;
pub mod registers;
mod room;
pub mod steuerbox;
mod storage;
mod tank;
pub mod thermal;
pub mod winter;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ems_core::prelude::{GeoPoint, Power, Slot};
use ems_forecast::ArrayModel;
use ems_sim::{BatterySim, PvSim};
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;

use crate::forecasting::Weather;
use crate::scenario::Scenario;
use registers::Unit;

fn allowed_power(
    home: &crate::site::Household,
    asset: &ems_core::prelude::AssetId,
    now: OffsetDateTime,
    requested: Power,
    grid: ems_core::prelude::Measurement,
) -> anyhow::Result<Power> {
    use anyhow::Context as _;
    use ems_core::prelude::{AssetTarget, Horizon, Measurement, Plan, Power, Slot, SlotPlan};
    use ems_realtime::{
        Arbiter, ArbiterConfig, Tick,
        guard::{GridLimits, SiteState},
    };
    use std::collections::BTreeMap;
    let state = SiteState {
        grid: Some(grid),
        assets: BTreeMap::from([(asset.clone(), Measurement::power(now, Power::ZERO))]),
        ..SiteState::default()
    };
    let mut plan = Plan::empty(Horizon::new(now, 1), now);
    plan.slots.push(SlotPlan {
        slot: Slot::containing(now),
        targets: vec![AssetTarget::fixed(asset.clone(), requested)],
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
        .get(asset)
        .copied()
        .context("gateway did not authorize an asset target")
}

/// The asset the § 14a limit applies to, as the reference household names it.
const CONNECTION_POINT: &str = "netzanschluss";

/// The Modbus unit of the inverter.
pub const UNIT_INVERTER: u8 = 1;
/// The Modbus unit of the meter at the grid connection.
pub const UNIT_METER: u8 = 2;
/// The Modbus unit of the battery.
pub const UNIT_BATTERY: u8 = 3;
const UNIT_TANK: u8 = 4;
const UNIT_ROOM: u8 = 5;
const UNIT_EV: u8 = 6;

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
    /// Optional appliances connected to this household meter.
    pub appliances: Appliances,
}

/// Installed virtual appliances in addition to the electrical plant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Appliances {
    /// Relay-driven hot water on unit 4.
    pub tank: bool,
    /// Heating-only room on unit 5.
    pub room: bool,
    /// Fixed three-phase wallbox on unit 6.
    pub ev: bool,
}

impl Appliances {
    fn site(self, scenario: &Scenario) -> crate::HouseholdConfig {
        crate::HouseholdConfig {
            evse: scenario
                .config
                .evse
                .filter(|_| self.ev)
                .map(|ev| crate::site::EvseConfig {
                    switchable: false,
                    bidirectional: false,
                    charge_limit: scenario.ev.map(|session| {
                        ems_core::prelude::Soc::clamped(
                            session.energy_target.get() / crate::scenario::VEHICLE_CAPACITY.get(),
                        )
                    }),
                    ..ev
                }),
            heat_pump: scenario.config.heat_pump.filter(|_| self.room).map(|hp| {
                crate::site::HeatPumpConfig {
                    control: ems_core::asset::HeatPumpControl::SgReady,
                    modulating: true,
                    cooling_electrical: None,
                    heating_rod: None,
                    ..hp
                }
            }),
            dhw: scenario.config.dhw.filter(|_| self.tank),
            dishwasher: None,
            ..scenario.config.clone()
        }
    }
}

/// The house's clock: the wall clock, moved onto the reference day.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    midnight: OffsetDateTime,
    offset: Duration,
    started: tokio::time::Instant,
    wall_start: OffsetDateTime,
}

impl Clock {
    fn new(scenario: &Scenario, at: Option<Duration>, now: OffsetDateTime) -> Self {
        let offset =
            at.unwrap_or_else(|| ems_core::calendar::to_berlin(now).time() - time::Time::MIDNIGHT);
        Self {
            midnight: scenario.start(),
            offset,
            started: tokio::time::Instant::now(),
            wall_start: now,
        }
    }

    /// How far into the reference day the house is.
    #[must_use]
    pub fn since_midnight(&self) -> Duration {
        self.offset + self.started.elapsed()
    }

    /// The instant of the reference day the house is living.
    #[must_use]
    pub fn now(&self) -> OffsetDateTime {
        self.midnight + self.since_midnight()
    }

    /// The instant of the reference day the house lives at wall-clock `real`.
    #[must_use]
    pub fn scenario_at(&self, real: OffsetDateTime) -> OffsetDateTime {
        self.midnight + self.offset + (real - self.wall_start)
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
    write_requests: u64,
    inverter: Unit,
    meter: Unit,
    battery: Unit,
    pv: PvSim,
    pack: Option<BatterySim>,
    tank: Option<tank::Tank>,
    room: Option<room::Room>,
    ev: Option<ev::Wallbox>,
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
        let pv = config
            .pv
            .ok_or_else(|| anyhow::anyhow!("the virtual house needs a roof"))?;
        let pack = config.battery.map(|battery| {
            let mut pack = BatterySim::new(battery.kwh, battery.power);
            pack.soc_min = battery.reserve_soc;
            pack.stored = battery.kwh * battery.reserve_soc.fraction().max(0.30);
            pack
        });
        Ok(Self {
            write_requests: 0,
            inverter: Unit::inverter(),
            meter: Unit::meter(),
            battery: Unit::battery(config.battery.map_or(Power::ZERO, |b| b.power)),
            pv: PvSim::default(),
            pack,
            tank: None,
            room: None,
            ev: None,
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

    fn install(
        &mut self,
        scenario: &Scenario,
        appliances: Appliances,
        loopback: bool,
    ) -> anyhow::Result<()> {
        if appliances.ev {
            anyhow::ensure!(loopback, "the wallbox fixture requires loopback");
            self.ev = Some(ev::Wallbox::new(scenario)?);
        }
        if appliances.room {
            anyhow::ensure!(loopback, "the space-heating fixture requires loopback");
            self.room = Some(room::Room::new(
                scenario
                    .config
                    .heat_pump
                    .ok_or_else(|| anyhow::anyhow!("missing heat pump"))?,
            )?);
        }
        if appliances.tank {
            anyhow::ensure!(loopback, "the relay-driven tank requires loopback");
            self.tank = Some(tank::Tank::new(&scenario.config)?);
        }
        Ok(())
    }

    /// Move every device on by `dt` at instant `at` of the reference day, and
    /// refresh the registers.
    ///
    /// The pack covers local demand until a model 704 command takes effect.
    /// Split a long step at reversion so the meter includes both regimes.
    pub fn step(&mut self, at: OffsetDateTime, dt: Duration) {
        self.step_with_load(at, dt, Power::ZERO);
    }

    fn step_with_load(&mut self, at: OffsetDateTime, dt: Duration, appliance: Power) {
        let target = self.battery.battery_target();
        if let Some((_, remaining)) = target
            && dt > remaining
        {
            self.step_with_load(at, remaining, appliance);
            self.step_with_load(at + remaining, dt - remaining, appliance);
            return;
        }
        let hours = dt.as_seconds_f64() / 3600.0;
        let available = self.weather.production_at(&self.array, self.location, at);
        let ceiling = self
            .inverter
            .curtailment()
            .map_or(self.rating, |fraction| self.rating * fraction);
        let (pv, curtailed) = self.pv.step(available, ceiling, dt);
        let produced = pv.outflow();
        let hot_water = self.tank.as_mut().map_or(Power::ZERO, |tank| {
            let slot = Slot::containing(at);
            let draw = self
                .weather
                .draw_in(slot, ems_forecast::hotwater::draw(slot))
                * (dt / ems_core::prelude::SLOT);
            tank.step(dt, draw)
        });
        let heating = self.room.as_mut().map_or(Power::ZERO, |room| {
            room.step(
                dt,
                self.weather.outdoor_at(at),
                self.weather
                    .window_at(self.location, at, crate::scenario::FACADE_AZIMUTH_DEG),
            )
        });
        let charging = self.ev.as_mut().map_or(Power::ZERO, |ev| ev.step(at, dt));
        let load = charging
            + heating
            + hot_water
            + appliance
            + self
                .weather
                .load_at(at, crate::scenario::household_load(Slot::containing(at)));
        let battery = self.pack.as_mut().map_or(Power::ZERO, |p| {
            p.step(target.map_or(produced - load, |(power, _)| power), dt)
        });
        if target.is_some() {
            self.battery.advance_battery(dt);
        }
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
            soc: self.pack.as_ref().map_or(0.0, |p| p.soc().fraction()),
        };
        self.inverter.set_inverter(
            produced.get(),
            self.produced_wh,
            100.0 * curtailed.get() / self.rating.get(),
        );
        self.meter
            .set_meter(grid.get(), self.imported_wh, self.exported_wh);
        self.battery
            .set_battery(self.reading.soc, self.reading.battery_w);
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
    appliances: Appliances,
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

    /// Count of write requests received by the simulated Modbus server, including refusals.
    #[must_use]
    pub fn write_requests(&self) -> u64 {
        self.physics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write_requests
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
/// port cannot be bound, or the scenario has no roof.
pub async fn start(options: Options) -> anyhow::Result<House> {
    if !options.listen.ip().is_loopback() && !options.exposed {
        anyhow::bail!(
            "the virtual house answers anyone who asks and has no credentials: \
             {} is not a loopback address (pass --exposed to mean it)",
            options.listen
        );
    }
    ensure_local_ship(options.household)?;
    let guard = options
        .household
        .map(|_| steuerbox::identity(options.identity.as_deref()))
        .transpose()?;
    let now = OffsetDateTime::now_utc();
    let clock = Clock::new(&options.scenario, options.at, now);
    let mut physics = Physics::new(&options.scenario)?;
    physics.install(
        &options.scenario,
        options.appliances,
        options.listen.ip().is_loopback(),
    )?;
    physics.step(clock.now(), Duration::ZERO);
    let physics = Arc::new(Mutex::new(physics));

    let listener = tokio::net::TcpListener::bind(options.listen).await?;
    let modbus = listener.local_addr()?;
    // The day's prices and sky, where `tariffd` and `forecastd` would answer
    // them, on the Modbus address's host and a port of its own.
    let (fleet, prices_and_sky) =
        fleet::serve(SocketAddr::new(modbus.ip(), 0), &options.scenario, clock).await?;
    let mut tasks = vec![
        prices_and_sky,
        tokio::spawn(modbus::serve(listener, Arc::clone(&physics))),
    ];

    let stepped = Arc::clone(&physics);
    tasks.push(tokio::spawn(async move {
        let second = std::time::Duration::from_secs(1);
        let mut every = tokio::time::interval_at(clock.started + second, second);
        loop {
            let tick = every.tick().await;
            let at = clock.midnight + clock.offset + (tick - clock.started) - Duration::SECOND;
            stepped
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .step(at, Duration::SECOND);
        }
    }));

    let (seen, steuerbox) = watch::channel(steuerbox::Seen::default());
    let silenced = Arc::new(AtomicBool::new(false));
    let mut steuerbox_ski = None;
    if let Some((household, identity)) = options.household.zip(guard) {
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
        appliances: options.appliances,
        silenced,
        tasks,
    })
}

fn ensure_local_ship(ship: Option<SocketAddr>) -> anyhow::Result<()> {
    anyhow::ensure!(
        ship.is_none_or(|listener| listener.ip().is_loopback()),
        "the virtual Steuerbox requires a loopback SHIP address"
    );
    Ok(())
}

/// Emit only the assets and site facts represented by this virtual plant.
fn wire_site(
    physical: &crate::HouseholdConfig,
    household: &crate::Household,
) -> anyhow::Result<toml::Table> {
    let (battery_kwh, battery_kw) = physical
        .battery
        .map_or((0.0, 0.0), |b| (b.kwh.kwh(), b.power.kw()));
    let declared: std::collections::BTreeMap<_, _> = physical
        .declared
        .iter()
        .filter(|(id, _)| {
            household
                .site
                .assets
                .iter()
                .any(|asset| asset.id().as_str() == id.as_str())
        })
        .map(|(id, facts)| (id, crate::config::DeclaredSettings::from(*facts)))
        .collect();
    let mut site = toml::Table::new();
    site.insert("declared".into(), toml::Value::try_from(declared)?);
    if let Some(pv) = physical.pv {
        site.insert(
            "para9".into(),
            toml::Value::try_from(crate::config::Para9Settings::from(pv.para9))?,
        );
    }
    if let Some(power) = physical.connection.contract_power {
        site.insert("contract_power_kw".into(), power.kw().into());
    }
    for (name, value) in [
        ("pv_kwp", physical.pv.map_or(0.0, |p| p.kwp.kw())),
        ("pv_ac_kw", physical.pv.map_or(0.0, |pv| pv.ac_nominal.kw())),
        ("pv_tilt_deg", physical.pv.map_or(0.0, |pv| pv.tilt_deg)),
        (
            "pv_azimuth_deg",
            physical.pv.map_or(0.0, |pv| pv.azimuth_deg),
        ),
        ("latitude", physical.location.latitude),
        ("longitude", physical.location.longitude),
        ("altitude_m", physical.location.altitude_m),
        ("fuse_a", physical.fuse.get()),
        (
            "reserve_soc",
            physical.battery.map_or(0.0, |b| b.reserve_soc.fraction()),
        ),
        (
            "evse_max_a",
            physical.evse.map_or(0.0, |ev| ev.max_current.get()),
        ),
        (
            "heat_pump_kw",
            physical.heat_pump.map_or(0.0, |hp| hp.power.kw()),
        ),
        ("dhw_litres", physical.dhw.map_or(0.0, |t| t.litres)),
        ("battery_kwh", battery_kwh),
        ("battery_kw", battery_kw),
        (
            "battery_wear_eur_per_kwh",
            physical.battery.map_or(0.0, |b| b.wear_eur_per_kwh),
        ),
    ] {
        site.insert(name.to_owned(), value.into());
    }
    site.insert("evse_switchable".into(), false.into());
    site.insert("evse_bidirectional".into(), false.into());
    if let Some(limit) = physical.evse.and_then(|ev| ev.charge_limit) {
        site.insert("ev_charge_limit".into(), limit.fraction().into());
    }
    if let Some(hp) = physical.heat_pump {
        site.insert("heat_pump_rod_kw".into(), 0.0.into());
        site.insert("heat_pump_control".into(), "sg-ready".into());
        site.insert("heat_pump_modulating".into(), true.into());
        site.insert("heat_pump_cooling_kw".into(), 0.0.into());
        site.insert("comfort_min_c".into(), hp.comfort_min_c.into());
        site.insert("comfort_max_c".into(), hp.comfort_max_c.into());
    }
    if let Some(tank) = physical.dhw {
        for (name, value) in [
            ("dhw_heater_kw", tank.heater.kw()),
            ("dhw_cop", tank.cop),
            ("dhw_standing_loss_w", tank.standing_loss.get()),
            ("dhw_t_min_c", tank.t_min_c),
            ("dhw_t_set_c", tank.t_set_c),
            ("dhw_t_max_c", tank.t_max_c),
        ] {
            site.insert(name.into(), value.into());
        }
    }
    site.insert("dishwasher_kw_steps".into(), toml::Value::Array(Vec::new()));
    Ok(site)
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
    manage_storage: bool,
) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    ensure_local_ship(ship)?;
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
    out.push_str(&device_configuration(
        scenario,
        house.modbus,
        manage_storage,
        house.appliances,
    )?);
    if house.appliances.ev {
        let session = scenario
            .ev
            .ok_or_else(|| anyhow::anyhow!("missing EV session"))?;
        let deadline = ems_core::calendar::to_berlin(
            house.clock.wall_start + session.departure - house.clock.offset,
        );
        writeln!(
            out,
            "\n[charging]\ndeparture = \"{:02}:{:02}\"\ntarget_soc = {}",
            deadline.hour(),
            deadline.minute(),
            session.energy_target.get() / crate::scenario::VEHICLE_CAPACITY.get()
        )?;
    }
    if let (Some(ship), Some(ski)) = (ship, &house.steuerbox_ski) {
        write!(
            out,
            "\n[ship]\nlisten = \"{ship}\"\ntrust = [\"{ski}\"]\nannounce = false\n"
        )?;
    }
    if ship.is_some() {
        write!(
            out,
            "\n[[drivers]]\nkind = \"eebus-lpc\"\nasset = \"{CONNECTION_POINT}\"\n"
        )?;
    }
    Ok(out)
}

fn device_configuration(
    scenario: &Scenario,
    modbus: SocketAddr,
    manage_storage: bool,
    appliances: Appliances,
) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    let physical = appliances.site(scenario);
    let household = crate::Household::build(&physical).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rating_kw = scenario.config.pv.map_or(0.0, |pv| pv.ac_nominal.kw());
    let mut out = String::new();
    let site = wire_site(&physical, &household)?;
    writeln!(
        &mut out,
        "\n{}",
        toml::to_string(&std::collections::BTreeMap::from([("site", site)]))?
    )?;
    let mut device = |asset: &str, unit: u8, extra: &str| {
        write!(
            out,
            "\n[[drivers]]\nkind = \"sunspec\"\nasset = \"{asset}\"\n\
             address = \"{modbus}\"\nunit = {unit}\n{extra}"
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
        device(
            battery.as_str(),
            UNIT_BATTERY,
            if manage_storage {
                "manage_storage = true\n"
            } else {
                ""
            },
        )?;
    }
    if let Some(ev) = &household.evse {
        ev::write_map(&mut out, ev, modbus)?;
    }
    for (asset, unit, command) in [
        (
            &household.dhw,
            UNIT_TANK,
            r#"command = "on_off", on = 1, off = 0"#,
        ),
        (
            &household.heat_pump,
            UNIT_ROOM,
            r#"command = "operation_mode", modes = [1, 2, 3]"#,
        ),
    ] {
        let Some(asset) = asset else { continue };
        writeln!(
            out,
            r#"
[[drivers]]
kind = "registers"
asset = "{asset}"
address = "{modbus}"
unit = {unit}
points = [
  {{ field = "temperature_c", register = 0, word = "s16", scale = 0.1 }},
  {{ field = "power", register = 1, word = "s16" }}
]
writes = [{{ register = 2, {command} }}]"#
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
            appliances: Appliances::default(),
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
        // The request starts at the real quarter boundary, before the process started.
        assert_eq!(
            prices["points"][0]["price_ct"],
            scenario.prices_ct
                [Slot::containing(house.clock.scenario_at(now)).index_in_local_day() as usize]
                .to_string()
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

        let written = configuration(
            &house,
            &scenario,
            None,
            "127.0.0.1:8080".parse().unwrap(),
            false,
        )
        .expect("a configuration");
        assert!(written.contains(&format!("tariffd_url = \"http://{}\"", house.fleet)));
    }

    #[tokio::test]
    async fn a_house_is_not_served_to_the_network_by_accident() {
        let refused = start(options("0.0.0.0:0")).await.expect_err("not loopback");
        assert!(refused.to_string().contains("--exposed"), "{refused}");
        let mut nonlocal_ship = options("127.0.0.1:0");
        nonlocal_ship.household = Some("192.0.2.1:4712".parse().unwrap());
        assert!(
            start(nonlocal_ship)
                .await
                .unwrap_err()
                .to_string()
                .contains("loopback SHIP")
        );
        let mut exposed = options("0.0.0.0:0");
        exposed.appliances.tank = true;
        exposed.exposed = true;
        assert!(
            start(exposed)
                .await
                .unwrap_err()
                .to_string()
                .contains("tank requires loopback")
        );
        let mut exposed_ev = options("0.0.0.0:0");
        exposed_ev.appliances.ev = true;
        exposed_ev.exposed = true;
        assert!(
            start(exposed_ev)
                .await
                .unwrap_err()
                .to_string()
                .contains("wallbox fixture requires loopback")
        );
        let mut exposed_room = options("0.0.0.0:0");
        exposed_room.appliances.room = true;
        exposed_room.exposed = true;
        assert!(
            start(exposed_room)
                .await
                .unwrap_err()
                .to_string()
                .contains("space-heating fixture requires loopback")
        );
        let house = start(options("127.0.0.1:0")).await.expect("loopback");
        assert!(
            configuration(
                &house,
                &options("127.0.0.1:0").scenario,
                Some("[::]:4712".parse().unwrap()),
                "127.0.0.1:8080".parse().unwrap(),
                false
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn the_configuration_it_writes_is_one_the_box_accepts() {
        let mut scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
        scenario.config.pv.as_mut().unwrap().tilt_deg = 22.0;
        scenario.config.location.altitude_m = 210.0;
        let house = start(Options {
            household: Some("127.0.0.1:4712".parse().expect("an address")),
            scenario: scenario.clone(),
            ..options("127.0.0.1:0")
        })
        .await
        .expect("a house");
        let written = configuration(
            &house,
            &scenario,
            Some("127.0.0.1:4712".parse().expect("an address")),
            "127.0.0.1:8080".parse().expect("an address"),
            false,
        )
        .expect("a configuration");
        let settings: crate::Settings = toml::from_str(&written).expect("it parses");
        assert_eq!(
            settings.drivers.len(),
            4,
            "three devices and the § 14a driver"
        );
        assert!(!settings.ship.announce, "a simulator's box stays off mDNS");
        let site = settings.site.household().unwrap();
        assert!(
            site.evse.is_none()
                && site.heat_pump.is_none()
                && site.dhw.is_none()
                && site.dishwasher.is_none()
        );
        assert_eq!(site.pv, scenario.config.pv);
        assert_eq!(site.battery, scenario.config.battery);
        assert_eq!(site.location, scenario.config.location);
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
