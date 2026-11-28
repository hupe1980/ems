//! A full electrical day on two independent virtual Modbus endpoints.
//! Scenario time advances at quarter boundaries. TCP and the SunSpec driver stay real.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration as Period;

use ems_core::prelude::Slot;
use ems_realtime::arbiter::{Arbiter, ArbiterConfig, CemRequest, Tick};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use time::{Duration, OffsetDateTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{Physics, modbus};
use crate::month::{IntervalTerms, MeteredBill};
use crate::runtime::control::Managed;
use crate::runtime::day::replay::{Capture, Comparison, Sample};
use crate::store::Recorded;
use crate::{HouseholdConfig, Running, Scenario};

struct Participant {
    physics: Arc<Mutex<Physics>>,
    running: Running,
    socket: TcpStream,
    listener: tokio::task::JoinHandle<()>,
    previous_kwh: [Decimal; 2],
}

impl Drop for Participant {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl Participant {
    async fn new(s: &Scenario, watch: bool) -> Self {
        Self::configured(s, watch, false).await
    }

    async fn configured(s: &Scenario, watch: bool, storage: bool) -> Self {
        Self::installed(s, watch, storage, super::Appliances::default()).await
    }

    async fn installed(
        s: &Scenario,
        watch: bool,
        storage: bool,
        appliances: super::Appliances,
    ) -> Self {
        let mut plant = Physics::new(s).unwrap();
        plant.install(s, appliances, true).unwrap();
        plant.step(s.start(), Duration::ZERO);
        let physics = Arc::new(Mutex::new(plant));
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let listener = tokio::spawn(modbus::serve(server, Arc::clone(&physics)));
        let socket = TcpStream::connect(address).await.unwrap();
        socket.set_nodelay(true).unwrap();
        let mut cfg = super::device_configuration(s, address, storage, appliances).unwrap();
        if !watch {
            cfg.push_str("\n[[drivers]]\nkind = \"eebus-lpc\"\nasset = \"netzanschluss\"\n");
        }
        let mut settings: crate::Settings = toml::from_str(&cfg).unwrap();
        if watch {
            settings.mode = crate::config::OperatingMode::Shadow;
            settings.external_limit_controller = Some("Independent test site limiter".into());
            settings.store_path = Some("shadow.redb".into());
        }
        if storage && !watch {
            let mut slow = settings.clone();
            slow.control.tick_period_s = 30;
            assert!(
                matches!(crate::runtime::assemble(&slow, None, s.start()), Err(crate::runtime::StartError::Build(e)) if e.contains("tick_period_s"))
            );
        }
        let running = crate::runtime::assemble(&settings, None, s.start()).unwrap();
        for (id, driver) in running.drivers.iter().zip(&settings.drivers) {
            if !matches!(
                driver,
                crate::config::DriverSettings::Sunspec(_)
                    | crate::config::DriverSettings::Registers(_)
            ) {
                continue;
            }
            running
                .registry
                .lock()
                .await
                .on_link(*id, ems_drv::LinkState::Up, s.start());
        }
        let mut arm = Self {
            physics,
            running,
            socket,
            listener,
            previous_kwh: [Decimal::ZERO; 2],
        };
        arm.exchange(s.start()).await;
        arm
    }

    async fn exchange(&mut self, now: OffsetDateTime) {
        let mut registry = self.running.registry.lock().await;
        registry.on_timeout(now);
        while let Some((id, request)) = registry.poll_transmit() {
            self.socket.write_all(&request).await.unwrap();
            let mut header = [0; 6];
            self.socket.read_exact(&mut header).await.unwrap();
            let size = usize::from(u16::from_be_bytes([header[4], header[5]]));
            let mut reply = header.to_vec();
            reply.resize(6 + size, 0);
            self.socket.read_exact(&mut reply[6..]).await.unwrap();
            registry.on_bytes(id, &reply, now).unwrap();
        }
    }

    async fn meter(
        &mut self,
        slot: Slot,
        now: OffsetDateTime,
    ) -> (crate::drivers::Observed, ems_grid::mispel::QuarterHour) {
        let observed = self
            .running
            .registry
            .lock()
            .await
            .observe(Some(&self.running.household.grid_meter), now);
        let meter = observed.state.grid.unwrap();
        let counters = [meter.energy_in.unwrap(), meter.energy_out.unwrap()]
            .map(|q| Decimal::from_f64(q.kwh()).unwrap().round_dp(3));
        let mut row = ems_grid::mispel::QuarterHour::empty(slot);
        row.grid_draw = counters[0] - self.previous_kwh[0];
        row.grid_feed_in = counters[1] - self.previous_kwh[1];
        self.previous_kwh = counters;
        (observed, row)
    }
}

fn managed(running: &Running) -> Managed {
    Managed {
        shadow: true,
        storage_control: None,
        can_replay: true,
        can_trace_storage: false,
        site: running.household.site.clone(),
        grid_meter: Some(running.household.grid_meter.clone()),
        undriven: Vec::new(),
        pv: running.household.pv.clone(),
        battery: None,
        evse: None,
        heat_pump: None,
        control_mode: ems_grid::para14a::ControlMode::Ems,
        modelled_pv: Arc::default(),
        bands: Arc::default(),
        outdoor: Arc::default(),
        window: Arc::default(),
    }
}

fn limit(
    arbiter: &Arbiter,
    site: &Managed,
    observed: &crate::drivers::Observed,
    now: OffsetDateTime,
    ceiling: ems_core::prelude::Power,
) -> ems_realtime::arbiter::Decision {
    let request = BTreeMap::from([(
        site.pv.clone().unwrap(),
        CemRequest::power(-ceiling, now + Duration::MINUTE),
    )]);
    arbiter.tick(Tick {
        now,
        site: &site.site,
        state: &observed.state,
        limits: &observed.limits,
        plan: None,
        overrides: &BTreeMap::new(),
        cem: &request,
        previous: &BTreeMap::new(),
        delivered: &BTreeMap::new(),
        phases: &BTreeMap::new(),
    })
}

async fn compare(scenario: &Scenario) -> bool {
    let (mut watched, mut driven) = tokio::join!(
        Participant::new(scenario, true),
        Participant::new(scenario, false)
    );
    let site = managed(&watched.running);
    let roof = site.pv.clone().unwrap();
    let rating = scenario.config.pv.unwrap().ac_nominal;
    let arbiter = Arbiter::new(ArbiterConfig::default());
    let mut capture = Capture::default();
    let mut observations = Vec::new();
    let mut metered = Vec::new();
    let day = ems_core::prelude::Horizon::new(scenario.start(), 96);
    let tariff = crate::site::tariff_for(&site.site, &scenario.prices_ct, day);
    let prices = ems_tariff::PriceStack::build(&tariff, day);
    for (index, slot) in day.slots().enumerate() {
        let now = slot.start() + Duration::SECOND;
        watched
            .physics
            .lock()
            .unwrap()
            .step(now, Duration::minutes(15));
        watched.exchange(now).await;
        let (observed, registers) = watched.meter(slot, now).await;
        // A mid-day request caps the inverter at an integral, representable percentage.
        // Other hours leave the roof at its full available output.
        let ceiling = if (44..60).contains(&index) {
            rating * 0.50
        } else {
            rating
        };
        let decision = limit(&arbiter, &site, &observed, now, ceiling);
        capture.observe(
            Sample::from_tick(&observed, &site, now, &decision),
            true,
            Period::from_mins(15),
        );
        let replay = capture.close().unwrap_or_else(|| {
            panic!(
                "missing replay {} quarter {index}, grid {:?} roof {:?} commanded {:?}",
                scenario.date,
                observed.state.grid,
                observed.state.asset(&roof),
                decision.commanded
            )
        });
        for sp in decision.setpoints.iter().filter(|sp| sp.asset == roof) {
            assert!(
                watched
                    .running
                    .registry
                    .lock()
                    .await
                    .command(sp, now)
                    .is_err()
            );
            driven
                .running
                .registry
                .lock()
                .await
                .command(sp, now)
                .unwrap();
        }
        // Complete command acknowledgments before advancing the independent driven point.
        driven.exchange(now).await;
        driven
            .physics
            .lock()
            .unwrap()
            .step(now, Duration::minutes(15));
        driven.exchange(now + Duration::seconds(2)).await;
        let (_, actual) = driven.meter(slot, now + Duration::seconds(2)).await;
        let price = prices.at(slot).unwrap();
        let terms = Some(IntervalTerms {
            contract: price.contract.clone(),
            import_ct: price.import_ct,
            export_ct: price.export_known.then_some(price.export_ct),
            annual_fixed_eur: price.annual_fixed_eur,
        });
        observations.push(Recorded {
            registers,
            production: None,
            terms: terms.clone(),
            replay: Some(replay),
            storage: None,
            watched: true,
        });
        metered.push(Recorded {
            registers: actual,
            production: None,
            terms,
            replay: None,
            storage: None,
            watched: false,
        });
    }
    assert_eq!(watched.physics.lock().unwrap().write_requests, 0);
    assert!(driven.physics.lock().unwrap().write_requests > 0);
    compare_bills(scenario.date, &observations, &metered)
}

fn compare_bills(date: time::Date, observations: &[Recorded], metered: &[Recorded]) -> bool {
    let bill = MeteredBill::from_registers(date, observations);
    let prediction = Comparison::close(observations, &bill).unwrap();
    let checkout = MeteredBill::from_registers(date, metered);
    let actual = checkout.energy_ct.unwrap() + checkout.fixed_ct.unwrap();
    // Model 203 rounds watts and cumulative Wh to integers; bills round once to cents.
    eprintln!(
        "{}: shadow {} ct, driven {actual} ct, measured replay error {} ct",
        date, prediction.with_plan_ct, prediction.error_ct
    );
    assert!(
        (actual - prediction.with_plan_ct).abs() <= prediction.error_ct + 1,
        "{}: predicted {} ct, independently metered {actual} ct, error {} ct",
        date,
        prediction.with_plan_ct,
        prediction.error_ct
    );
    metered.iter().zip(observations).any(|(driven, watched)| {
        (driven.registers.grid_feed_in - watched.registers.grid_feed_in).abs() > Decimal::new(1, 2)
    })
}

#[tokio::test]
async fn matched_socket_costs_on_every_reference_day() {
    let config = HouseholdConfig::default();
    let scenarios = [
        Scenario::winter_with_grid_event(config.clone()),
        Scenario::summer_surplus(config.clone()),
        Scenario::winter_evening_deadline(config.clone()),
        Scenario::winter_evening_no_store(&config),
        Scenario::summer_without_a_planner(config.clone()),
        Scenario::autumn_without_a_planner(config.clone()),
        Scenario::summer_capped(&config),
    ];
    let mut changed_export = false;
    for mut scenario in scenarios {
        // The trial isolates the supported fixed-load/PV topology on each weather and price series.
        scenario.config.battery = None;
        changed_export |= tokio::time::timeout(Period::from_secs(30), compare(&scenario))
            .await
            .unwrap();
    }
    assert!(
        changed_export,
        "a curtailment must change the separately metered export"
    );
}

/// Compare the connection-meter driver with both the plant balance and a
/// separately stepped minute reference. All three use the self-managed pack.
#[tokio::test]
async fn berlin_meter_days_reconcile_on_the_wire() {
    let mut scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
    for (date, quarters) in [
        (scenario.date, 96),
        (time::macros::date!(2026 - 03 - 29), 92),
        (time::macros::date!(2026 - 10 - 25), 100),
    ] {
        scenario.date = date;
        reconcile_meter_day(&scenario, quarters).await;
    }
}

async fn reconcile_meter_day(scenario: &Scenario, quarters: usize) {
    use rust_decimal::prelude::ToPrimitive as _;
    let mut wire = Participant::new(scenario, true).await;
    let mut reference = Physics::new(scenario).unwrap();
    let mut total_wh = [0.0; 2];
    let mut expected_charge_ct = 0.0_f64;
    let mut rows = Vec::with_capacity(quarters);
    let mut max_quarter_error = 0.0_f64;
    let day = ems_core::prelude::Horizon::new(scenario.start(), quarters);
    for quarter in day.slots() {
        let mut balance_wh = [0.0; 2];
        {
            let mut plant = wire.physics.lock().unwrap();
            for second in 0..900 {
                let at = quarter.start() + Duration::seconds(second);
                plant.step(at, Duration::SECOND);
                let r = plant.reading;
                let net = r.load_w + r.battery_w - r.pv_w;
                balance_wh[0] += net.max(0.0) / 3600.0;
                balance_wh[1] += (-net).max(0.0) / 3600.0;
                if second % 60 == 0 {
                    reference.step(at, Duration::MINUTE);
                }
            }
        }
        wire.exchange(quarter.end()).await;
        let (_, registers) = wire.meter(quarter, quarter.end()).await;
        assert_eq!(
            registers.slot.start(),
            quarter.end() - Duration::minutes(15)
        );
        assert_eq!(
            ems_core::calendar::to_berlin(registers.slot.start()).date(),
            scenario.date
        );
        for (index, kwh) in [registers.grid_draw, registers.grid_feed_in]
            .into_iter()
            .enumerate()
        {
            let wh = kwh.to_f64().unwrap() * 1000.0;
            let error = (wh - balance_wh[index]).abs();
            assert!(
                error <= 1.000_001,
                "{quarter}, direction {index}: {wh} Wh vs {} Wh",
                balance_wh[index]
            );
            max_quarter_error = max_quarter_error.max(error);
            total_wh[index] += wh;
        }
        let import_ct = if ems_core::calendar::to_berlin(quarter.start()).hour() >= 18 {
            40
        } else {
            25
        };
        expected_charge_ct += (balance_wh[0] * f64::from(import_ct) - balance_wh[1] * 8.0) / 1000.0;
        rows.push(Recorded {
            registers,
            terms: Some(IntervalTerms {
                contract: None,
                import_ct: import_ct.into(),
                export_ct: Some(8.into()),
                annual_fixed_eur: Decimal::ZERO,
            }),
            production: None,
            replay: None,
            storage: None,
            watched: true,
        });
    }
    let bill = MeteredBill::from_registers(scenario.date, &rows);
    assert_eq!(usize::from(bill.priced_intervals), quarters);
    let charge_ct = bill.energy_ct.expect("every quarter is priced");
    assert_eq!(bill.fixed_ct, Some(0));
    assert!(charge_ct.abs_diff(expected_charge_ct.round().to_i64().unwrap()) <= 1);
    assert_eq!(
        day.slots().last().unwrap().end(),
        ems_core::calendar::day_end_utc(scenario.date)
    );
    assert_eq!(wire.physics.lock().unwrap().write_requests, 0);
    let native = [reference.imported_wh, reference.exported_wh];
    assert!(total_wh[0] > 0.0);
    for (wire_wh, reference_wh) in total_wh.into_iter().zip(native) {
        let tolerance = 50.0_f64.max(reference_wh * 0.01);
        assert!(
            (wire_wh - reference_wh).abs() <= tolerance,
            "wire {wire_wh} Wh vs minute reference {reference_wh} Wh"
        );
    }
    eprintln!(
        "{} · {quarters} quarters · wire import/export {:.0}/{:.0} Wh · minute reference {:.1}/{:.1} Wh · maximum quarter register error {max_quarter_error:.3} Wh · fixture bill {charge_ct} ct",
        scenario.date, total_wh[0], total_wh[1], native[0], native[1]
    );
}

/// Run the same four physical assets independently. The negative control leaves
/// the EV unplugged from its charger during the requested charging window.
fn mixed_service_frames(
    s: &Scenario,
    charge_at: Option<Duration>,
) -> Vec<crate::scenario::service::Frame> {
    use crate::scenario::CONTROL_PERIOD as dt;
    use ems_core::prelude::{Energy, Power};
    use ems_sim::{BatterySim, BuildingSim, EvseSim, TankSim, VehicleSim};

    let ev = s.ev.unwrap();
    let mut port = EvseSim::new().with_vehicle(VehicleSim {
        capacity: Energy::from_kwh(60.0),
        stored: ev.energy_now,
        efficiency: 0.92,
        efficiency_single_phase: 0.88,
        max_charge: Power::from_kw(11.0),
    });
    let mut battery = BatterySim::new(Energy::from_kwh(10.0), Power::from_kw(3.0));
    let mut tank = TankSim::new(Energy::from_kwh(6.0), Power::from_kw(2.0));
    let mut house = BuildingSim::new(21.0);
    let mut deficit = Energy::ZERO;
    let mut at = s.start();
    let end = ems_core::calendar::day_end_utc(s.date);
    let mut frames = Vec::new();
    while at < end {
        let elapsed = at - s.start();
        let charging = elapsed >= ev.arrival
            && elapsed < ev.departure
            && charge_at.is_some_and(|from| elapsed >= from && elapsed < from + Duration::HOUR);
        let car = port.step(
            if charging {
                Power::from_kw(4.2)
            } else {
                Power::ZERO
            },
            dt,
        );
        let heat = house.step(Power::from_kw(2.0), 15.0, 0.0, dt);
        let draw = if elapsed.whole_minutes() == 30 {
            Energy::from_kwh(0.5)
        } else {
            Energy::ZERO
        };
        let (water, short) = tank.step(
            if tank.fill() < 0.6 {
                tank.heater
            } else {
                Power::ZERO
            },
            draw,
            dt,
        );
        deficit += short;
        let store = battery.step(
            if elapsed < Duration::HOUR {
                Power::new(500.0)
            } else {
                Power::ZERO
            },
            dt,
        );
        let indoor = house.indoor_c();
        frames.push(crate::scenario::service::Frame {
            at,
            grid: Power::new(250.0) + car + heat + water + store,
            outside_band_k: (19.0 - indoor).max(indoor - 25.0).max(0.0),
            water_short: deficit,
            vehicle: port.vehicle.as_ref().map(|v| v.stored),
            battery: battery.stored,
            tank: tank.stored,
            programme_done: true,
            limited: false,
        });
        at += dt;
    }
    frames
}

async fn wire_registers(
    s: &Scenario,
    frames: &[crate::scenario::service::Frame],
) -> Vec<ems_grid::mispel::QuarterHour> {
    let mut peer = Participant::new(s, true).await;
    let mut totals = [0.0; 2];
    let mut rows = Vec::new();
    let (quarters, remainder) = frames.as_chunks::<15>();
    assert!(remainder.is_empty());
    for group in quarters {
        for frame in group {
            totals[0] += frame.grid.inflow().over(Duration::MINUTE).get();
            totals[1] += frame.grid.outflow().over(Duration::MINUTE).get();
        }
        let last = group.last().unwrap();
        peer.physics
            .lock()
            .unwrap()
            .meter
            .set_meter(last.grid.get(), totals[0], totals[1]);
        peer.exchange(last.at).await;
        let (_, row) = peer.meter(Slot::containing(group[0].at), last.at).await;
        rows.push(row);
    }
    assert_eq!(peer.physics.lock().unwrap().write_requests, 0);
    rows
}

#[tokio::test]
async fn mixed_service_requires_a_complete_equivalent_day_before_pricing() {
    use crate::runtime::day::replay::GapReason;
    use crate::scenario::service::close;
    use ems_core::prelude::{Energy, Horizon};
    let mut s = Scenario::winter_with_grid_event(HouseholdConfig::default());
    s.grid_event = None;
    s.dishwasher = None;
    s.ev = Some(crate::EvPlan {
        energy_now: Energy::from_kwh(10.0),
        energy_target: Energy::from_kwh(12.0),
        arrival: Duration::HOUR,
        departure: Duration::hours(3),
    });
    let planned = mixed_service_frames(&s, Some(Duration::HOUR));
    let mut baseline = mixed_service_frames(&s, Some(Duration::HOUR));
    let missed = mixed_service_frames(&s, None);
    let (meter, other, cheaper) = tokio::join!(
        wire_registers(&s, &planned),
        wire_registers(&s, &baseline),
        wire_registers(&s, &missed)
    );
    assert_eq!(meter, other);
    let mut prices = ems_tariff::PriceStack::build(
        &ems_tariff::Tariff::fixed(30.into(), 0.into()),
        Horizon::new(s.start(), 96),
    );
    for price in &mut prices.slots[8..12] {
        price.import_ct = 10.into();
    }
    let equal = close(&s, &planned, &baseline, &meter, &prices, None);
    let c = equal.comparison.unwrap();
    assert_eq!(c.with_plan_ct, c.without_plan_ct);
    assert!(equal.gap.is_none());
    let shifted = mixed_service_frames(&s, Some(Duration::hours(2)));
    let shifted_meter = wire_registers(&s, &shifted).await;
    let shifted_trial = close(&s, &shifted, &baseline, &shifted_meter, &prices, None);
    let saving = shifted_trial.comparison.unwrap();
    assert!(saving.delta() > i128::from(saving.error_ct));
    assert!((saving.without_plan_ct - c.with_plan_ct).abs() <= 1);
    assert_eq!(
        saving.with_plan_ct,
        shifted_trial.bill.energy_ct.unwrap() + shifted_trial.bill.fixed_ct.unwrap()
    );
    let failed = close(&s, &missed, &baseline, &cheaper, &prices, None);
    assert!(failed.bill.energy_ct.unwrap() < equal.bill.energy_ct.unwrap());
    assert!(failed.comparison.is_none());
    assert_eq!(failed.gap.unwrap().reason, GapReason::VehicleEnergy);

    for reason in [
        GapReason::ThermalComfort,
        GapReason::HotWater,
        GapReason::BatteryInventory,
    ] {
        let mut damaged = planned.clone();
        match reason {
            GapReason::ThermalComfort => damaged[90].outside_band_k = 1.0,
            GapReason::HotWater => damaged[30].water_short = Energy::new(50.0),
            _ => damaged.last_mut().unwrap().battery += Energy::new(2.0),
        }
        let trial = close(&s, &damaged, &baseline, &meter, &prices, None);
        assert_eq!(trial.gap.unwrap().reason, reason);
        assert!(trial.comparison.is_none());
    }
    let before_loss = baseline[42].vehicle.take();
    let missing_charge = close(&s, &planned, &baseline, &meter, &prices, None);
    assert_eq!(
        missing_charge.gap.unwrap().at_unix_s,
        baseline[42].at.unix_timestamp()
    );
    assert!(missing_charge.comparison.is_none());
    baseline[42].vehicle = before_loss;
    let lost = baseline.remove(42);
    let missing = close(&s, &planned, &baseline, &meter, &prices, None);
    assert!(missing.comparison.is_none());
    assert_eq!(missing.gap.unwrap().at_unix_s, lost.at.unix_timestamp());

    for (date, count) in [
        (time::macros::date!(2026 - 03 - 29), 92),
        (time::macros::date!(2026 - 10 - 25), 100),
    ] {
        s.date = date;
        let frames = mixed_service_frames(&s, Some(Duration::HOUR));
        let registers = wire_registers(&s, &frames).await;
        let prices = ems_tariff::PriceStack::build(
            &ems_tariff::Tariff::fixed(30.into(), 0.into()),
            Horizon::new(s.start(), count),
        );
        let trial = close(&s, &frames, &frames, &registers, &prices, None);
        assert_eq!(usize::from(trial.bill.priced_intervals), count);
        assert!(trial.comparison.is_some());
    }
}

async fn battery_controller(
    address: std::net::SocketAddr,
) -> (tokio_modbus::client::Context, u16, u16) {
    use tokio_modbus::prelude::{Reader as _, Slave};
    let mut controller =
        tokio_modbus::client::tcp::connect_slave(address, Slave(super::UNIT_BATTERY))
            .await
            .unwrap();
    let mut header = super::registers::BASE + 2;
    loop {
        let words = controller
            .read_holding_registers(header, 2)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(words[0], 0xffff, "battery control model absent");
        if words[0] == 704 {
            return (controller, header + 2, words[1]);
        }
        header += 2 + words[1];
    }
}

async fn set_battery_watts(
    controller: &mut tokio_modbus::client::Context,
    enable: u16,
    watts: i32,
) {
    use tokio_modbus::prelude::Writer as _;
    let [a, b, c, d] = watts.to_be_bytes();
    controller
        .write_multiple_registers(
            enable,
            &[1, 1, u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d])],
        )
        .await
        .unwrap()
        .unwrap();
}

/// An independent Modbus controller drives unit 3 while the gateway's own
/// meter driver observes the resulting flow at unit 2.
#[tokio::test]
async fn signed_battery_power_reverts_on_the_wire() {
    use sunspec::Model as _;
    use sunspec::models::model704::{Model704, WSetEna, WSetMod};
    use tokio_modbus::prelude::{Reader as _, Writer as _};
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
    let mut wire = Participant::new(&scenario, true).await;
    let (mut controller, body, length) = battery_controller(wire.socket.peer_addr().unwrap()).await;
    let enable = body + Model704::W_SET_ENA.offset;
    let mut now = scenario.start();
    for watts in [-2000_i32, 1500] {
        set_battery_watts(&mut controller, enable, watts).await;
        let reading = {
            let mut plant = wire.physics.lock().unwrap();
            plant.step(now, Duration::SECOND);
            plant.reading
        };
        now += Duration::SECOND;
        assert!((reading.battery_w + f64::from(watts)).abs() < 0.01);
        wire.exchange(now).await;
        let observed = wire
            .running
            .registry
            .lock()
            .await
            .observe(Some(&wire.running.household.grid_meter), now);
        let grid = observed.state.grid.unwrap().power.unwrap().get();
        assert!((grid - (reading.load_w - reading.pv_w - f64::from(watts))).abs() <= 0.5);
        let words = controller
            .read_holding_registers(body, length)
            .await
            .unwrap()
            .unwrap();
        let active = Model704::parse(&words).unwrap();
        assert_eq!(active.w_set_mod, Some(WSetMod::Watts));
        assert_eq!(active.w_set_ena, Some(WSetEna::Enabled));
        assert_eq!(active.w_set_sf, Some(0));
        assert_eq!(active.w_set, Some(watts));
        assert_eq!(active.w_set_rvrt_rem, Some(29));
        for invalid in [&[1, 1, 0x7fff, 0xffff][..], &[1, 1, 0], &[1, 0, 0, 0]] {
            let refused = controller
                .write_multiple_registers(enable, invalid)
                .await
                .unwrap();
            assert_eq!(refused, Err(tokio_modbus::ExceptionCode::IllegalDataValue));
        }
        assert_eq!(
            controller
                .read_holding_registers(body, length)
                .await
                .unwrap()
                .unwrap(),
            words
        );
    }
    {
        let mut plant = wire.physics.lock().unwrap();
        let pack = plant.pack.as_mut().unwrap();
        pack.stored = pack.capacity * pack.soc_min.fraction();
        plant.step(now, Duration::SECOND);
        assert!(
            plant.reading.battery_w.abs() < 0.01,
            "reserve blocks discharge"
        );
        plant.step(now + Duration::SECOND, Duration::seconds(29));
    }
    let expired = Model704::parse(
        &controller
            .read_holding_registers(body, length)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(expired.w_set_ena, Some(WSetEna::Disabled));
    assert_eq!(expired.w_set_rvrt_rem, Some(0));
    // A single plant step can straddle expiry. Charge for 30 seconds, then
    // cover the house autonomously during the final second.
    set_battery_watts(&mut controller, enable, -2000).await;
    {
        let mut plant = wire.physics.lock().unwrap();
        let start = now + Duration::seconds(31);
        let before = plant.imported_wh;
        plant.step(start, Duration::seconds(31));
        assert!(plant.reading.battery_w <= 0.0);
        let charged_wh = plant.imported_wh - before;
        assert!(charged_wh >= 2000.0 * 30.0 / 3600.0);
    }
    set_battery_watts(&mut controller, enable, 500).await;
    controller
        .write_single_register(enable, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        controller
            .read_holding_registers(enable, 4)
            .await
            .unwrap()
            .unwrap(),
        [0, 1, 0, 0]
    );
}

/// Exercise the commissioned gateway driver, its measured pack power and its bounded lease.
#[tokio::test]
async fn gateway_storage_trial_renews_only_on_fresh_decisions() {
    use ems_core::prelude::Power;
    let arbiter = Arbiter::new(ArbiterConfig::default());
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
    let mut wire = Participant::configured(&scenario, false, true).await;
    let battery = wire.running.household.battery.clone().unwrap();
    let grid = wire.running.household.grid_meter.clone();
    // Let the first physical sample reach the gateway before requesting discharge.
    let start = scenario.start() + Duration::SECOND;
    let mut confirmations = Vec::new();
    for second in 0..=100 {
        let at = start + Duration::seconds(second);
        wire.physics.lock().unwrap().step(at, Duration::SECOND);
        wire.exchange(at).await;
        let wanted = if second < 2 { -1500.0 } else { 2000.0 };
        if second <= 66 {
            let observed = wire.running.registry.lock().await.observe(Some(&grid), at);
            let decision = storage_decision(&wire.running, &observed, &arbiter, at, wanted);
            let sp = decision
                .setpoints
                .iter()
                .find(|sp| sp.asset == battery)
                .unwrap();
            wire.running.registry.lock().await.command(sp, at).unwrap();
            wire.exchange(at).await;
        }
        let observed = wire.running.registry.lock().await.observe(Some(&grid), at);
        let (_, receipt) = observed.storage_feedback.unwrap();
        if second <= 66 {
            assert_eq!(
                receipt.confirmed.map(Power::get),
                Some(wanted),
                "second {second}"
            );
            let at = receipt.last_confirmed.unwrap();
            if confirmations.last() != Some(&at) {
                confirmations.push(at);
            }
        }
        if (3..=66).contains(&second) || second == 1 {
            let pack = observed.state.asset(&battery).unwrap().power.unwrap().get();
            assert_eq!(pack, wanted);
            let plant = wire.physics.lock().unwrap().reading;
            assert!(
                (observed.state.grid.unwrap().power.unwrap().get()
                    - (plant.load_w - plant.pv_w + pack))
                    .abs()
                    <= 1.0
            );
        }
        if second == 100 {
            assert!(receipt.autonomous && receipt.confirmed.is_none());
        }
    }
    let before = wire.physics.lock().unwrap().write_requests;
    assert_eq!(
        before,
        u64::try_from(confirmations.len()).unwrap(),
        "each write had a readback; expiry generated no writes"
    );
    assert!(confirmations.len() >= 6 && confirmations.len() <= 8);
    for pair in confirmations.windows(2) {
        assert!(pair[1] - pair[0] <= Duration::seconds(15));
    }
    verify_reserve_and_release(&mut wire, start + Duration::seconds(101)).await;
}

fn storage_decision(
    running: &Running,
    observed: &crate::drivers::Observed,
    arbiter: &Arbiter,
    now: OffsetDateTime,
    watts: f64,
) -> ems_realtime::Decision {
    let battery = running.household.battery.as_ref().unwrap();
    let request = BTreeMap::from([(
        battery.clone(),
        CemRequest::power(ems_core::prelude::Power::new(watts), now),
    )]);
    crate::runtime::control::decide(
        arbiter,
        Tick {
            now,
            site: &running.household.site,
            state: &observed.state,
            limits: &observed.limits,
            plan: None,
            overrides: &BTreeMap::new(),
            cem: &request,
            previous: &BTreeMap::new(),
            delivered: &BTreeMap::new(),
            phases: &BTreeMap::new(),
        },
        Some(battery),
    )
}

async fn verify_reserve_and_release(wire: &mut Participant, at: OffsetDateTime) {
    use ems_core::prelude::{Command, Power, RealtimeCause, Reason, Setpoint};
    let battery = wire.running.household.battery.clone().unwrap();
    let grid = wire.running.household.grid_meter.clone();
    let discharge = Setpoint::new(
        battery.clone(),
        Command::ActivePower(Power::new(-1500.0)),
        Reason::Realtime(RealtimeCause::SurplusTracking),
        at,
    )
    .unwrap();
    wire.running
        .registry
        .lock()
        .await
        .command(&discharge, at)
        .unwrap();
    wire.exchange(at).await;
    {
        let mut plant = wire.physics.lock().unwrap();
        let pack = plant.pack.as_mut().unwrap();
        pack.stored = pack.capacity * pack.soc_min.fraction();
        plant.step(at, Duration::SECOND);
    }
    let at = at + Duration::SECOND;
    wire.exchange(at).await;
    let observed = wire.running.registry.lock().await.observe(Some(&grid), at);
    assert_eq!(
        observed
            .storage_feedback
            .as_ref()
            .unwrap()
            .1
            .confirmed
            .unwrap()
            .get(),
        -1500.0
    );
    assert_eq!(
        observed.state.asset(&battery).unwrap().power.unwrap().get(),
        0.0,
        "pack delivered no power at reserve"
    );
    let arbiter = Arbiter::new(ArbiterConfig::default());
    let mut missing_soc = observed.clone();
    missing_soc.state.assets.get_mut(&battery).unwrap().soc = None;
    let mut stale_grid = observed;
    stale_grid.state.grid.as_mut().unwrap().at -= Duration::HOUR;
    for input in [&missing_soc, &stale_grid] {
        let decision = storage_decision(&wire.running, input, &arbiter, at, 2000.0);
        assert!(!decision.commanded.contains_key(&battery));
        let release = decision
            .setpoints
            .iter()
            .find(|s| s.asset == battery)
            .unwrap();
        assert_eq!(release.command, Command::OnOff(false));
        wire.running
            .registry
            .lock()
            .await
            .command(release, at)
            .unwrap();
    }
    wire.exchange(at).await;
    assert!(
        wire.running
            .registry
            .lock()
            .await
            .observe(Some(&grid), at)
            .storage_feedback
            .unwrap()
            .1
            .autonomous
    );
}

/// Move only the start of charging. Both households retain the same thermal
/// equipment, weather, outlet rating, departure and initial storage.
#[tokio::test]
async fn a_winter_departure_is_visible_on_the_occupied_house_meter() {
    let scenario = Scenario::winter_with_grid_event(HouseholdConfig::default());
    let (early, late) = tokio::join!(occupied_day(&scenario, 90), occupied_day(&scenario, 120));
    assert_eq!(early.temperatures, late.temperatures);
    assert_eq!(early.temperatures.len(), 96);
    assert!((early.outlet_wh - 22_770.0).abs() <= 1.0);
    assert!((late.outlet_wh - 20_700.0).abs() <= 1.0);
    assert!(early.vehicle_wh >= 38_000.0 && late.vehicle_wh < 38_000.0);
    eprintln!(
        "96 Berlin quarters per arm · 07:00 departure: {:.0} vs {:.0} outlet Wh; {:.0} vs {:.0} stored vehicle Wh · identical room and tank readings at all quarter closes",
        early.outlet_wh, late.outlet_wh, early.vehicle_wh, late.vehicle_wh
    );
}

struct OccupiedDay {
    temperatures: Vec<(f64, f64)>,
    outlet_wh: f64,
    vehicle_wh: f64,
}

async fn occupied_day(scenario: &Scenario, start_minute: i64) -> OccupiedDay {
    use ems_core::prelude::{Command, Current, Reason, Setpoint, UserOverride};
    let mut wire = Participant::installed(
        scenario,
        false,
        false,
        super::Appliances {
            room: true,
            tank: true,
            ev: true,
        },
    )
    .await;
    let household = &wire.running.household;
    let (ev, room, tank) = (
        household.evse.clone().unwrap(),
        household.heat_pump.clone().unwrap(),
        household.dhw.clone().unwrap(),
    );
    let mut temperatures = Vec::new();
    let mut outlet = None;
    let mut stored = None;
    let start = scenario.start();
    for minute in (0..24 * 60).step_by(15) {
        let at = start + Duration::minutes(minute);
        if minute == start_minute {
            wire.running
                .registry
                .lock()
                .await
                .command(
                    &Setpoint {
                        asset: ev.clone(),
                        command: Command::ChargingCurrent(Current::new(6.0)),
                        at,
                        reason: Reason::User(UserOverride::Boost),
                    },
                    at,
                )
                .unwrap();
            wire.exchange(at).await;
        }
        let mut integrated = [0.0; 2];
        {
            let mut plant = wire.physics.lock().unwrap();
            for second in 0..900 {
                plant.step(at + Duration::seconds(second), Duration::SECOND);
                let net = plant.reading.load_w + plant.reading.battery_w - plant.reading.pv_w;
                integrated[0] += net.max(0.0) / 3600.0;
                integrated[1] += (-net).max(0.0) / 3600.0;
            }
        }
        let close = at + Duration::minutes(15);
        wire.exchange(close).await;
        let (observed, registers) = wire.meter(Slot::containing(at), close).await;
        for (metered, calculated) in [registers.grid_draw, registers.grid_feed_in]
            .iter()
            .zip(integrated)
        {
            assert!(
                (*metered * Decimal::from(1000) - Decimal::from_f64(calculated).unwrap()).abs()
                    <= Decimal::ONE
            );
        }
        let devices = &observed.state.assets;
        assert_eq!(devices[&room].at, close);
        assert_eq!(devices[&tank].at, close);
        temperatures.push((
            devices[&room].temperature_c.unwrap(),
            devices[&tank].temperature_c.unwrap(),
        ));
        let reading = devices[&ev];
        let presence = observed.vehicles[&ev];
        let deadline = scenario.ev.unwrap().departure;
        assert_eq!(presence.connected, close < start + deadline);
        let wh = reading.energy_in.unwrap().get();
        if close == start + deadline {
            outlet = Some(wh);
            stored =
                Some(reading.soc.unwrap().fraction() * crate::scenario::VEHICLE_CAPACITY.get());
            let expected = scenario.ev.unwrap().energy_now.get() + wh * 0.92;
            // Vehicle charge is encoded at 0.01% of the 60 kWh capacity.
            assert!((stored.unwrap() - expected).abs() <= 3.0);
        }
        if close >= start + deadline {
            assert_eq!(reading.power.unwrap().get(), 0.0);
            assert_eq!(outlet, Some(wh));
        }
    }
    OccupiedDay {
        temperatures,
        outlet_wh: outlet.unwrap(),
        vehicle_wh: stored.unwrap(),
    }
}
