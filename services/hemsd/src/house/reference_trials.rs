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
        let physics = Arc::new(Mutex::new(Physics::new(s).unwrap()));
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let listener = tokio::spawn(modbus::serve(server, Arc::clone(&physics)));
        let socket = TcpStream::connect(address).await.unwrap();
        socket.set_nodelay(true).unwrap();
        let roof = s.config.pv.unwrap();
        let cfg = format!(
            r#"
[site]
pv_kwp = {dc}
pv_ac_kw = {ac}
battery_kwh = 0
evse_max_a = 0
heat_pump_kw = 0
dhw_litres = 0
dishwasher_kw_steps = []
[site.declared.pv]
commissioned = "2019-05-01"
[[drivers]]
kind = "sunspec"
asset = "pv"
address = "{address}"
unit = 1
rating_kw = {ac}
[[drivers]]
kind = "sunspec"
asset = "netzanschluss-zaehler"
address = "{address}"
unit = 2
listens_only = true
"#,
            dc = roof.kwp.kw(),
            ac = roof.ac_nominal.kw()
        );
        let mut settings: crate::Settings = toml::from_str(&cfg).unwrap();
        if watch {
            settings.mode = crate::config::OperatingMode::Shadow;
            settings.external_limit_controller = Some("Independent test site limiter".into());
            settings.store_path = Some("shadow.redb".into());
        }
        let running = crate::runtime::assemble(&settings, None, s.start()).unwrap();
        for id in &running.drivers {
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
        let mut row = ems_grid::mispel::QuarterHour::empty(Slot::containing(now));
        row.grid_draw = counters[0] - self.previous_kwh[0];
        row.grid_feed_in = counters[1] - self.previous_kwh[1];
        self.previous_kwh = counters;
        (observed, row)
    }
}

fn managed(running: &Running) -> Managed {
    Managed {
        shadow: true,
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
        let (observed, registers) = watched.meter(now).await;
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
        let (_, actual) = driven.meter(now + Duration::seconds(2)).await;
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
