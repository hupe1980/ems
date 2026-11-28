//! `hemsd` — the hems edge daemon.

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use hemsd::{HouseholdConfig, Scenario};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "hemsd", version, about = "The hems edge daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare risk policies across weather realizations for one reference day.
    Risk {
        /// Which day to run.
        #[arg(long, value_enum, default_value_t = Day::Winter)]
        day: Day,
        /// Number of weather realizations.
        #[arg(long, default_value_t = 4)]
        days: usize,
        /// Report as JSON rather than a table.
        #[arg(long)]
        json: bool,
    },
    /// Score the forecast band across independent weather realizations.
    Backtest {
        /// Which day to run.
        #[arg(long, value_enum, default_value_t = Day::Summer)]
        day: Day,
        /// Number of weather realizations.
        #[arg(long, default_value_t = 20)]
        days: usize,
        /// Report as JSON rather than a table.
        #[arg(long)]
        json: bool,
    },
    /// Calculate a local seller sheet's gross import price for one quarter hour.
    PriceSheet {
        /// Price sheet to inspect offline.
        file: PathBuf,
        /// Verify a compact Ed25519 JWS and print its public-key fingerprint.
        #[arg(long)]
        signed: bool,
        /// Start of the quarter hour as an RFC 3339 timestamp with offset.
        #[arg(long)]
        at: String,
        /// Published net index for this slot (ct/kWh), required for a dynamic contract.
        #[arg(long, allow_hyphen_values = true)]
        index_ct: Option<String>,
    },
    /// Check the configuration, device responses, pairing and gateway clock.
    Doctor {
        /// Configuration of the running household gateway.
        #[arg(long, env = "EMS_HEMSD_CONFIG")]
        config: PathBuf,
        /// PEM certificate for a gateway using a private trust root.
        #[arg(long)]
        ca_cert: Option<PathBuf>,
    },
    /// Run the household gateway with its configured devices.
    Run {
        /// Household TOML file containing the site and driver list.
        #[arg(long, env = "EMS_HEMSD_CONFIG")]
        config: Option<PathBuf>,
        /// Check the configuration, build the site and the drivers, and exit
        /// without opening a socket.
        #[arg(long)]
        check: bool,
    },
    /// Read the configured Matter appliance once and print its power observation as JSON.
    #[cfg(feature = "matter")]
    MatterRead {
        /// Household configuration containing the commissioned point.
        #[arg(long, env = "EMS_HEMSD_CONFIG")]
        config: PathBuf,
    },
    /// Replay a reference day through the household page, on this machine.
    ///
    /// The day is simulated once through the real guard, arbiter and planner,
    /// then served minute by minute at `http://127.0.0.1:8080` — the page a box
    /// on a wall serves, with a slider through the day. No configuration, no
    /// devices, no credential.
    Demo {
        /// Which day to replay.
        #[arg(long, value_enum, default_value_t = Day::Winter)]
        day: Day,
        /// Where to listen. Loopback only: the demo asks for no credential.
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: std::net::SocketAddr,
        /// How many earlier days to simulate as well, so the month has closed
        /// days to add up. At most ninety — what a box keeps.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u16).range(0..=90))]
        days: u16,
    },
    /// Replay a Berlin winter electrical day with storage and morning S2 and OCPP service.
    #[cfg(feature = "house")]
    WinterDay {
        /// Export a standalone report with a table of metered quarters.
        #[arg(long, conflicts_with = "json")]
        html: bool,
        /// Export both sets of timestamped cumulative register readings.
        #[arg(long)]
        json: bool,
        /// Usable heat required at the end of the morning water draw, in Wh.
        #[arg(long, default_value_t = 2300, value_parser = clap::value_parser!(u32).range(0..=20000))]
        target_wh: u32,
        /// Energy requested at the outlet by departure, in Wh.
        #[arg(long, default_value_t = 3600, value_parser = clap::value_parser!(i32).range(1..=10000))]
        departure_wh: i32,
    },
    /// Run a winter hot-water trial over loopback S2 and print the peer observations.
    #[cfg(feature = "house")]
    ThermalTrial {
        /// Continue on the same meter through a 45-minute OCPP departure with this outlet goal.
        #[arg(long, value_parser = clap::value_parser!(i32).range(1..=10000))]
        departure_wh: Option<i32>,
        /// Disconnect the managed controller after its first five-minute lease.
        #[arg(long)]
        disconnect: bool,
        /// Render a standalone HTML receipt instead of JSON.
        #[arg(long)]
        html: bool,
        /// Withhold the final sensor observation.
        #[arg(long)]
        missing_close: bool,
        /// Water promised at the deadline, in Wh of usable heat.
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u32).range(0..=20000))]
        target_wh: u32,
    },
    #[cfg(feature = "house")]
    #[command(hide = true)]
    ThermalPeer {
        #[arg(long)]
        missing_close: bool,
    },
    /// Rehearse a household departure with an OCPP wallbox that loses power for 15 minutes.
    #[cfg(feature = "house")]
    EvDeparture {
        /// Energy wanted at the charging outlet by the end of the 45-minute session, in Wh.
        #[arg(long, default_value_t = 3600, value_parser = clap::value_parser!(i32).range(1..=10000))]
        goal_wh: i32,
        /// Disconnect before the departure register reaches the controller.
        #[arg(long)]
        missing_close: bool,
        /// Send the signed simulation receipt to this obsd origin, using EMS_OBSD_SECRET.
        #[arg(long)]
        report_to: Option<String>,
        /// Site whose key signs the receipt.
        #[arg(long, default_value = "reference-household")]
        site: String,
    },
    /// Be a household on the wire: an inverter, a meter and a battery over
    /// Modbus TCP and a Steuerbox over EEBUS, for `hemsd run` to manage.
    ///
    /// Writes the box's configuration, then lives a reference day in real time.
    /// With `--run`, the box that manages it starts in the same process;
    /// otherwise, in a second terminal: `hemsd run --config house.toml`.
    #[cfg(feature = "house")]
    House {
        /// Which day the house lives.
        #[arg(long, value_enum, default_value_t = Day::Winter)]
        day: Day,
        /// Where in that day to start, `HH:MM`; the wall clock's time otherwise.
        /// `--at 16:58` puts the winter day's § 14a reduction two minutes away.
        #[arg(long, value_parser = time_of_day)]
        at: Option<time::Duration>,
        /// Where the devices answer Modbus TCP.
        #[arg(long, default_value = "127.0.0.1:1502")]
        listen: std::net::SocketAddr,
        /// Loopback address where the box listens for the virtual Steuerbox (SHIP).
        #[arg(long, default_value = "127.0.0.1:4712")]
        ship: std::net::SocketAddr,
        /// Where the box will serve its page and API.
        #[arg(long, default_value = "127.0.0.1:8080")]
        api: std::net::SocketAddr,
        /// The box's configuration, written here. The Steuerbox's certificate
        /// is kept beside it so the SKI in the file stays true.
        #[arg(long, default_value = "house.toml")]
        out: PathBuf,
        /// Retain gateway quarters and closed-day reports beside the configuration.
        #[arg(long)]
        keep_history: bool,
        /// Start the box that manages the house as well — the real product, in
        /// one command.
        #[arg(long)]
        run: bool,
        /// Include a hot-water tank with measured temperature, power and a writable relay.
        #[arg(long)]
        with_tank: bool,
        /// Include the occupied-room sensor and three-state heat pump.
        #[arg(long)]
        with_room: bool,
        /// Connect the scenario vehicle to a three-phase Modbus wallbox on loopback.
        #[arg(long)]
        with_ev: bool,
        /// Authorize the gateway to control the loopback battery with bounded watt commands.
        #[arg(long)]
        manage_storage: bool,
        /// Allow `--listen` on an address other machines can reach. The house
        /// asks nobody for a credential.
        #[arg(long)]
        exposed: bool,
    },
    /// Print the box's HTTP API as an OpenAPI 3.1 document.
    Openapi {
        /// Print the reference page's table of operations instead.
        #[arg(long)]
        markdown: bool,
    },
    /// Run one simulated day through the whole control stack and report on it.
    Simulate {
        /// Which day to run.
        #[arg(long, value_enum, default_value_t = Day::Winter)]
        day: Day,
        /// Read installed site and asset ratings from a gateway TOML configuration.
        #[arg(long, value_name = "GATEWAY.toml")]
        site_from: Option<PathBuf>,
        /// Report as JSON rather than a table.
        #[arg(long)]
        json: bool,
        /// Plan with an empty household history, as on first installation.
        #[arg(long)]
        cold_start: bool,
        /// Price battery wear at this many euros per kilowatt-hour of
        /// throughput. Zero reproduces a cost-only optimiser.
        #[arg(long)]
        wear_eur_per_kwh: Option<f64>,
        /// Wire the charge point to three fixed conductors, so it cannot drop to
        /// one when the surplus is too small for a three-phase session.
        #[arg(long)]
        no_phase_switching: bool,
        /// Run as though an intelligent metering system with a control device
        /// were in operation, which lifts the § 9 Abs. 2 EEG 60 % feed-in cap.
        #[arg(long)]
        imsys: bool,
        /// Fit a single-speed heat pump rather than a modulating one.
        ///
        /// The only configuration in which a minimum runtime constrains
        /// anything: the compressor is at its rating or off, so cycling is a
        /// decision the planner makes and the day can count. A modulating unit
        /// — what most German households have, and what every other reference
        /// day runs — has nothing to start.
        #[arg(long)]
        heat_pump_on_off: bool,
        /// Hand the planner the exact series the simulator is about to run.
        ///
        /// A comparison, never a default: the difference between this and an
        /// ordinary run is what forecast error costs a household. Any saving
        /// quoted from it is an upper bound no box in a real house can reach.
        #[arg(long)]
        perfect_foresight: bool,
        /// How the planner treats the fact that its forecasts are wrong.
        #[arg(long, value_enum, default_value_t = Risk::Median)]
        risk: Risk,
        /// Price carbon dioxide at this many euros per kilogram, so the plan
        /// prefers the hours the grid is clean and not only the hours it is
        /// cheap.
        ///
        /// Zero — the default — is the plain economic plan. 55 €/t, the German
        /// price for heating and transport fuels, is `0.055`. The intensity
        /// itself comes from the price stack, so what this buys is a plan that
        /// moves load towards clean hours and a figure for what that cost.
        #[arg(long)]
        co2_eur_per_kg: Option<f64>,
        /// Pay this many euros per kilowatt-hour to avoid taking one from the
        /// grid at all — the self-sufficiency dial.
        ///
        /// It is honest about its price: near the day's spread it makes the plan
        /// prefer its own roof even where importing would be marginally
        /// cheaper, and the difference is what independence cost. Somebody who
        /// bought a battery for autarky wants exactly that, and it belongs in
        /// the objective rather than in a marketing figure.
        #[arg(long)]
        autarky_eur_per_kwh: Option<f64>,
        /// Give every asset the same allocation weight.
        ///
        /// Without per-asset shadow prices the guard's *weighted* max-min
        /// allocator is handed one number for the whole slot and weights
        /// nothing: a car three hours from its departure and a heat pump in a
        /// warm house get equal shares of a § 14a reduction.
        #[arg(long)]
        uniform_weights: bool,
        /// Report the day to an `obsd` at this URL rather than only printing it.
        ///
        /// The fleet's own view of a household is a `DayKpis` — a dozen numbers
        /// rather than the whole report — and this is the producer of it. Without
        /// one, `obsd` is a service with no caller, which is the failure mode
        /// this workspace keeps finding in itself.
        #[arg(long, env = "EMS_OBSD_URL")]
        report_to: Option<String>,
        /// The secret the box and the fleet share, for the Standard Webhooks
        /// signature over the report.
        ///
        /// Required with `--report-to`, because a fleet view that accepts an
        /// unsigned day is a fleet view anybody who can reach it may write to —
        /// and what they would be writing is the list of households that did not
        /// respect a network operator's reduction.
        #[arg(long, env = "EMS_OBSD_SECRET")]
        report_secret: Option<String>,
        /// Keep the day's § 14a evidence and quarter-hour registers in a local
        /// store at this path, `[A1 7.2]` and `[A1 7.3]`.
        ///
        /// The box's **own** two years. The house is never worse off when the
        /// cloud is gone, and a record that exists only once it has
        /// been uploaded is an intention with a network dependency — so the box
        /// records first and forwards second, and what has not been
        /// acknowledged is the store's outbox.
        #[arg(long, env = "HEMS_STORE")]
        store: Option<PathBuf>,
        /// The site identifier to report under.
        #[arg(long, default_value = "reference-household")]
        site: String,
        /// Put the household in a § 42c energy-sharing community: three roofs'
        /// worth of neighbours' array, an equal third of the
        /// Aufteilungsschlüssel, electricity at 12 ct/kWh net.
        ///
        /// § 42c EnWG has applied since 01.06.2026. The comparison is the point:
        /// the same day run with and without it says what a community is worth
        /// to a household that can *move* its load into the hours the community
        /// is generating — which is the whole behavioural reason to join one.
        #[arg(long)]
        sharing: bool,
    },
}

/// How the planner treats forecast error.
#[derive(Clone, Copy, clap::ValueEnum)]
enum Risk {
    /// One future: the median of both forecasts, priced as though it were
    /// certain. What every deterministic energy manager does.
    Median,
    /// Three futures from the band the forecast already carries, minimising the
    /// **expected** cost across them.
    Expected,
    /// The same three futures, with a third of the objective on the worst of
    /// them — a household that would rather not be caught out.
    Hedged,
    /// One median, and **three futures on the days that need them** — decided by
    /// how much slack the charging session has left.
    ///
    /// Over twenty seeded weathers on each of two days it comes within €0,04 of
    /// always planning against three futures on the evening that needs them and
    /// within €0,11 of the median on the day that does not — level with the
    /// median in money over the two, and delivering the service the median
    /// leaves short. Not the default, because it costs four times the solve to
    /// do it, which is a household's trade rather than an inherited one.
    Adaptive,
}

impl Risk {
    /// The starting policy, before any re-solve.
    fn model(self) -> ems_optimizer::Risk {
        match self {
            Risk::Median | Risk::Adaptive => ems_optimizer::Risk::deterministic(),
            Risk::Expected => ems_optimizer::Risk::expected(),
            Risk::Hedged => ems_optimizer::Risk::hedged(),
        }
    }

    /// Whether a plan that says a service is at risk is re-solved against three
    /// futures.
    const fn adaptive(self) -> bool {
        matches!(self, Risk::Adaptive)
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Day {
    /// A January day with a § 14a reduction from 17:00 to 18:30.
    Winter,
    /// A June day with more production than the house can use, including four
    /// quarter hours of negative prices.
    Summer,
    /// A January evening with a § 14a reduction and a car that has to charge
    /// through it.
    Deadline,
    /// The same evening on a household with no store, where the reduction has
    /// to be shared and the allocation weights decide who gets it.
    Shared,
    /// The June day with the planner switched off — the box on its own.
    Offline,
    /// A September day with the planner off and broken cloud, where the surplus
    /// spends its time in the band only one conductor can use.
    Autumn,
    /// The June day with nobody home: the § 9 EEG 60 % cap against a roof the
    /// house cannot use.
    Capped,
}

/// Minutes of a local day as `HH:MM`, for the one line an installer reads back
/// against a price sheet.
fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
}

/// The box on the wall: configuration in, sockets open, guard and arbiter
/// running until somebody stops it.
///
/// # What it refuses to start with
///
/// Everything the registry can catch is caught before a byte moves (D75), and
/// `--check` is that half on its own — the command an installer runs before
/// leaving the cellar. The alternative is a box that comes up, looks healthy and
/// is never told about a reduction, which is the failure this daemon exists to
/// make impossible.
type Installer = std::sync::Arc<hemsd::runtime::installation::Installer>;

async fn manage(
    config: Option<&std::path::Path>,
    check: bool,
    installer: Option<Installer>,
) -> anyhow::Result<()> {
    let settings: hemsd::Settings = ems_service::load(config, "EMS_HEMSD")?;
    if let (Some(path), Some(installer)) = (config, &installer)
        && installer.empty_site()?
    {
        return hemsd::runtime::first_run::run(path, &settings, installer.clone()).await;
    }
    ems_service::init_tracing(
        ems_service::identity!(),
        &settings.service.log_filter,
        settings.service.log_json,
    );

    #[cfg(feature = "matter")]
    let meter = settings
        .matter
        .as_ref()
        .map(hemsd::runtime::matter::Meter::new)
        .transpose()?;
    if check {
        let now = time::OffsetDateTime::now_utc();
        // No store: `run --check` validates the *configuration*, and reaching
        // into the box's record would make the answer depend on what an
        // operator happened to have written — which is exactly the thing the
        // installer is not checking here.
        let running = hemsd::runtime::assemble(&settings, None, now)?;
        // The SKI, printed rather than only logged: it is what an installer has
        // to give the metering point operator before a Steuerbox can be told to
        // trust this box, and it is the step field reports say goes wrong most
        // often. Deriving it here also *creates* it on a first run, so the
        // number does not change when the daemon starts for real.
        // The store is opened once for both credentials. Deriving them here
        // *creates* them on a first run, which is the point: an installer doing
        // a dry run carries away the same two numbers the daemon will serve
        // with, rather than two that change when it starts for real.
        let store = match &settings.store_path {
            Some(path) => Some(std::sync::Arc::new(tokio::sync::Mutex::new(
                hemsd::store::Store::open(path)?,
            ))),
            None => None,
        };
        if settings.ship.listen.is_some() {
            let (_, ski, _key) =
                hemsd::runtime::ship::identity(&settings.ship, store.as_ref(), now).await?;
            println!("🔑 SKI  {}", ski.to_display_string());
            println!("   give this to the metering point operator, so the Steuerbox trusts it");
        }
        // The other credential, and the one without which every surface below
        // answers `401`.
        let site = running.household.site.id.to_string();
        let issued = match &store {
            Some(store) => {
                let held = store.lock().await;
                hemsd::runtime::access::LocalAccess::resolve(&settings.api, &site, Some(&held))?.1
            }
            None => hemsd::runtime::access::LocalAccess::resolve(&settings.api, &site, None)?.1,
        };
        if let Some(announcement) = issued.announcement() {
            println!("{announcement}");
        }
        let now = time::OffsetDateTime::now_utc();
        let modes = std::collections::BTreeMap::new();
        let described = ems_flex::describe_site(
            &running.household.site,
            &ems_flex::DescribeContext::new(now, now + time::Duration::hours(24), &modes),
        );
        println!(
            "✅ {} assets, {} drivers, {} resources described in S2{}",
            running.household.site.assets.len(),
            settings.drivers.len(),
            described.resources.len(),
            match described.undescribed.len() {
                0 => String::new(),
                n => format!(", {n} it cannot express"),
            }
        );
        // The one thing an installer transcribes by hand from a PDF, and
        // therefore the one most likely to be a typo. `assemble` has already
        // refused a calendar that breaks the Anwendungshilfe; this says which
        // one passed, and where it came from, so the check leaves a record of
        // the document the household will be billed against.
        if let Some(m) = &settings.tariff.modul3 {
            println!(
                "📅 Modul 3 `{}` for {} conforms — HT {}–{}, NT {}–{}, billed in {}",
                m.id,
                m.year,
                clock(m.hochtarif_minutes[0]),
                clock(m.hochtarif_minutes[1]),
                clock(m.niedertarif_minutes[0]),
                clock(m.niedertarif_minutes[1]),
                m.billed_quarters.join(", "),
            );
            if let Some(source) = &m.source {
                println!("   transcribed from {source}");
            }
        }
        return Ok(());
    }

    let (signal, trigger) = ems_service::Shutdown::channel();
    tokio::spawn(ems_service::shutdown::on_signal(trigger.clone()));

    let health = ems_service::Health::new();
    // Not ready until a driver has actually been heard from. A box that reported
    // itself ready while every device was silent would be reporting that the
    // *process* was up, which is what liveness is for.
    health.bad("drivers", "no driver has reported yet");

    let running = hemsd::runtime::run(&settings, &health, &signal).await?;
    let site = running.household.site.id.to_string();
    // The second credential a commissioning visit carries away. The SKI goes to
    // the metering point operator so a Steuerbox trusts this box; this one goes
    // to whoever reads the box's own screens, and without it every surface below
    // answers `401`.
    if let Some(announcement) = &running.api_token {
        println!("{announcement}");
    }

    // The household's own surface, and — where the household has connected an
    // energy manager — the one that manager drives it over. Both on the socket
    // the shell already binds: one port to configure, one to firewall, and a
    // metrics label that is the route rather than every asset's name.
    let https = running.tls.clone();
    if let Some((_, fingerprint)) = &https {
        println!("  HTTPS certificate (SHA-256): {fingerprint}");
    }
    // Web Push needs somewhere to keep a signing key and the subscriptions, so
    // a box with no store has none — and answers `/v1/push` with `404`.
    let push = match &running.store {
        Some(store) => Some(hemsd::runtime::push::Push::open(store.clone()).await?),
        None => None,
    };
    let local = hemsd::runtime::api::Local::new(
        running.status,
        site,
        running.ski,
        running.overrides,
        running.trust,
        running.access.clone(),
        running.series,
    )
    .with_days(running.store.clone())
    .with_home(hemsd::runtime::api::HomeSources {
        health: health.clone(),
        site: Some(std::sync::Arc::new(running.household.site.clone())),
        plan: running.plan.clone(),
        prices: running.prices.clone(),
        goal: running.goal.clone(),
        tariff_changed: running.tariff_changed.clone(),
        car: running.car.clone(),
        tank: running.tank.clone(),
        last: running.last.clone(),
        fingerprint: running.fingerprint.clone(),
        https_fingerprint: https.as_ref().map(|(_, f)| f.clone()),
    })
    .with_commissioning(hemsd::runtime::handover::Commissioning::of(&settings))
    .with_settings(settings.clone())
    .with_registry(std::sync::Arc::clone(&running.registry))
    .with_push(push.clone());
    let local = if let Some(point) = &settings.heat_meter {
        local.with_heat_meter(hemsd::runtime::heat_meter::Observer::start(
            point,
            signal.clone(),
        )?)
    } else {
        local
    };
    let _control_box_gauge = local.publish_control_box_health()?;
    #[cfg(feature = "matter")]
    let local = if let Some(meter) = meter {
        let reader = meter.clone();
        let stop = signal.clone();
        health.vital("matter", signal.clone(), async move {
            ems_service::shutdown::until(stop, reader.poll()).await;
        });
        local.with_matter(meter)
    } else {
        local
    };
    // Events out: kept in the store where there is one, and in memory where
    // there is not — a box without a store can still tell Home Assistant.
    let hooks = hemsd::runtime::webhooks::Webhooks::new(
        &running.household.site.id.to_string(),
        running.store.clone(),
    );
    let local = local.with_webhooks(Some(hooks.clone()));
    health.vital(
        "webhooks",
        signal.clone(),
        hemsd::runtime::webhooks::run(hooks.clone(), signal.clone()),
    );
    {
        let watched = local.clone();
        health.vital(
            "detector",
            signal.clone(),
            hemsd::runtime::push::run(
                push,
                Some(hooks),
                move || {
                    let watched = watched.clone();
                    async move {
                        let home = watched.document().await?;
                        serde_json::to_value(home).ok()
                    }
                },
                signal.clone(),
            ),
        );
    }
    let local = if let Some(installer) = &installer {
        // A stored or configured credential survives the handover to the candidate.
        if settings.store_path.is_some()
            || (settings.api.token.is_some() && settings.ship.listen.is_none())
        {
            local.with_installer(installer.clone(), trigger.clone())
        } else {
            local
        }
    } else {
        local
    };
    let router = hemsd::runtime::surfaces_over(
        local,
        settings.s2.enabled.then(|| {
            hemsd::runtime::s2::Surface::new(
                std::sync::Arc::new(running.household.site.clone()),
                std::sync::Arc::clone(&running.registry),
                running.cem.clone(),
                settings.s2.clone(),
            )
        }),
        running.access.clone(),
        https.is_some(),
    );

    let server = ems_service::Server::new(
        ems_service::identity!(),
        settings.service.clone(),
        health,
        router,
    );
    let server = match https {
        Some((config, _)) => server.with_tls(config),
        None => server,
    };
    let listener = tokio::net::TcpListener::bind(settings.service.listen).await?;
    let served = server.serve_on(listener, signal);
    tokio::pin!(served);
    if let Some(installer) = installer
        && let Some(assets) = installer.trial_assets()?
    {
        tokio::select! {
            result = &mut served => return Ok(result?),
            () = confirm_device(assets, installer, running.registry, trigger, settings.control.tick_period_s) => {}
        }
    }
    served.await?;
    Ok(())
}

async fn confirm_device(
    roles: Vec<hemsd::runtime::installation::DeviceRole>,
    installer: Installer,
    registry: hemsd::runtime::Shared,
    stop: ems_service::shutdown::ShutdownTrigger,
    tick_seconds: u64,
) {
    let wait = async {
        loop {
            {
                let state = registry
                    .lock()
                    .await
                    .state(None, time::OffsetDateTime::now_utc());
                if roles.iter().all(|role| {
                    state.assets.iter().any(|(id, measurement)| {
                        id.as_str() == role.asset() && role.measured(measurement)
                    })
                }) {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    };
    let limit = std::time::Duration::from_secs(tick_seconds.saturating_mul(3).clamp(30, 180));
    if tokio::time::timeout(limit, wait).await.is_ok() && installer.confirm().is_ok() {
        return;
    }
    if let Err(error) = installer.rollback() {
        tracing::error!(%error, "could not restore the previous configuration");
    } else {
        installer.request_restart();
    }
    stop.trigger();
}

fn main() -> anyhow::Result<()> {
    let command = Cli::parse().command;
    if let Command::Run {
        config,
        check: false,
    } = &command
    {
        use std::io::ErrorKind;
        let path = config
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("hemsd.toml"));
        if !path.exists() {
            let mut open = std::fs::OpenOptions::new();
            open.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(0o600);
            }
            match open.open(path) {
                Ok(file) => file.sync_all()?,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let installer = match hemsd::runtime::installation::Installer::open(path) {
            Ok(installer) => Some(std::sync::Arc::new(installer)),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
                ) && !path.with_added_extension("apply.json").exists() =>
            {
                None
            }
            Err(error) => return Err(error.into()),
        };
        loop {
            let result = {
                let runtime = tokio::runtime::Runtime::new()?;
                if let Some(installer) = &installer {
                    if !runtime.block_on(installer.candidate_ready())? {
                        tracing::warn!(
                            "a reviewed device changed or is unavailable; restoring the previous configuration"
                        );
                        installer.rollback()?;
                    }
                    installer.begin()?;
                }
                runtime.block_on(manage(Some(path), false, installer.clone()))
            };
            let Some(installer) = &installer else {
                return result;
            };
            if result.is_err()
                && installer
                    .receipt()?
                    .is_some_and(|r| r.outcome == hemsd::runtime::installation::Outcome::Restarting)
            {
                if let Err(error) = &result {
                    tracing::warn!(%error, "candidate gateway failed to start; restoring previous configuration");
                }
                installer.rollback()?;
                continue;
            }
            if !installer.take_restart() {
                return result;
            }
        }
    }
    tokio::runtime::Runtime::new()?.block_on(execute(command))
}

async fn execute(command: Command) -> anyhow::Result<()> {
    // The long-running gateway takes its tracing settings from the household configuration.
    if !matches!(command, Command::Run { .. }) {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "hemsd=info".into()),
            )
            .init();
    }

    match command {
        Command::PriceSheet {
            file,
            signed,
            at,
            index_ct,
        } => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(file)?
                .take(if signed {
                    hemsd::price_sheet::MAX_ENVELOPE as u64 + 1
                } else {
                    65_537
                })
                .read_to_end(&mut bytes)?;
            let (sheet, signer) = if signed {
                let verified = hemsd::price_sheet::verify(&bytes)?;
                (
                    verified.sheet,
                    Some(
                        serde_json::json!({"kid":verified.key_id,"fingerprint":verified.fingerprint}),
                    ),
                )
            } else {
                (ems_tariff::sheet::PriceSheet::parse(&bytes)?, None)
            };
            let instant =
                time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)?;
            let slot = ems_core::prelude::Slot::starting_at(instant)
                .ok_or_else(|| anyhow::anyhow!("--at must start on a quarter-hour boundary"))?;
            let index = index_ct
                .as_deref()
                .map(rust_decimal::Decimal::from_str_exact)
                .transpose()?;
            let price = sheet.quote(slot, index)?;
            println!(
                "{}",
                serde_json::json!({
                    "contract": sheet.contract, "signer":signer, "at": at, "index":sheet.index,
                    "published_components":sheet.published_components,
                    "gross_import_ct_per_kwh":price.import_ct.normalize().to_string(),
                })
            );
        }
        Command::Doctor { config, ca_cert } => {
            return hemsd::doctor::run(&config, ca_cert.as_deref()).await;
        }
        Command::Run { config, check } => return manage(config.as_deref(), check, None).await,
        #[cfg(feature = "matter")]
        Command::MatterRead { config } => {
            anyhow::ensure!(config.is_file(), "household configuration file is missing");
            let settings: hemsd::Settings = ems_service::load(Some(&config), "EMS_HEMSD")?;
            let point = settings.matter.as_ref().ok_or_else(|| {
                anyhow::anyhow!("add a [matter] point to the household configuration")
            })?;
            let meter = hemsd::runtime::matter::Meter::new(point)?;
            meter.refresh().await?;
            println!("{}", serde_json::to_string(&meter.view())?);
        }
        Command::Openapi { markdown } => {
            print!(
                "{}",
                if markdown {
                    hemsd::openapi::markdown()
                } else {
                    hemsd::openapi::json()
                }
            );
        }
        Command::Demo { day, listen, days } => {
            hemsd::demo::loopback_only(listen)?;
            let scenario = scenario_for(day, HouseholdConfig::default());
            println!(
                "  simulating {} through the guard, the arbiter and the planner…",
                scenario.date
            );
            if days > 0 {
                println!("  …and the {days} days before it, for the month");
            }
            let demo = hemsd::demo::Demo::record_with_history(&scenario, usize::from(days))?;
            println!("  open http://{listen}  — the household page, replaying the day");
            return hemsd::demo::serve(demo, listen).await;
        }
        #[cfg(feature = "house")]
        Command::WinterDay {
            html,
            json,
            target_wh,
            departure_wh,
        } => {
            let receipt = hemsd::house::winter::trial(target_wh, departure_wh).await?;
            if html {
                println!("{}", receipt.html());
            } else if json {
                println!("{}", serde_json::to_string_pretty(&receipt)?);
            } else {
                println!("{receipt}");
            }
        }
        #[cfg(feature = "house")]
        Command::ThermalTrial {
            departure_wh,
            disconnect,
            missing_close,
            target_wh,
            html,
        } => {
            let receipt =
                hemsd::house::thermal::trial(target_wh, missing_close, disconnect, departure_wh)
                    .await?;
            println!(
                "{}",
                if html {
                    receipt.html()
                } else {
                    serde_json::to_string_pretty(&receipt)?
                }
            );
        }
        #[cfg(feature = "house")]
        Command::ThermalPeer { missing_close } => {
            return hemsd::house::thermal::peer(missing_close).await;
        }
        #[cfg(feature = "house")]
        Command::EvDeparture {
            goal_wh,
            missing_close,
            report_to,
            site,
        } => {
            let receipt = hemsd::house::departure::rehearse(goal_wh, missing_close).await?;
            if let Some(origin) = report_to {
                let secret = std::env::var("EMS_OBSD_SECRET")
                    .context("set EMS_OBSD_SECRET to the site's signing key")?;
                receipt
                    .publish(&origin, &site, &ems_events::webhook::parse_secret(&secret)?)
                    .await?;
            }
            println!("{receipt}");
        }
        #[cfg(feature = "house")]
        Command::House {
            day,
            at,
            listen,
            ship,
            api,
            out,
            keep_history,
            run,
            manage_storage,
            with_tank,
            with_room,
            with_ev,
            exposed,
        } => {
            return house(HouseArgs {
                day,
                at,
                listen,
                ship,
                api,
                out,
                keep_history,
                exposed,
                run,
                manage_storage,
                with_tank,
                with_room,
                with_ev,
            })
            .await;
        }
        Command::Simulate {
            day,
            site_from,
            json,
            cold_start,
            wear_eur_per_kwh,
            no_phase_switching,
            imsys,
            heat_pump_on_off,
            perfect_foresight,
            risk,
            uniform_weights,
            co2_eur_per_kg,
            autarky_eur_per_kwh,
            report_to,
            report_secret,
            store,
            site,
            sharing,
        } => {
            let mut config = if let Some(path) = site_from {
                #[derive(serde::Deserialize)]
                struct Installed {
                    site: hemsd::config::SiteSettings,
                }
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading installed site from {}", path.display()))?;
                toml::from_str::<Installed>(&text)
                    .with_context(|| format!("loading [site] from {}", path.display()))?
                    .site
                    .household()?
            } else {
                HouseholdConfig::default()
            };
            if let (Some(wear), Some(battery)) = (wear_eur_per_kwh, &mut config.battery) {
                battery.wear_eur_per_kwh = wear;
            }
            if let Some(evse) = &mut config.evse {
                evse.switchable &= !no_phase_switching;
            }
            if let Some(heat_pump) = &mut config.heat_pump {
                heat_pump.modulating &= !heat_pump_on_off;
            }
            if imsys && let Some(pv) = &mut config.pv {
                // The network operator's first successful Ansteuerbarkeit test
                // — which is what § 9 Abs. 2 EEG actually waits for, and the
                // only thing this flag changes. The intelligent metering system
                // itself has been in since 2024 on both sides of the comparison,
                // so § 51's negative quarter hours are held constant and what
                // moves is the 60 % cap alone.
                pv.para9.relief = ems_core::prelude::CapRelief::ImsysWithControl;
            }
            let mut scenario = scenario_for(day, config);
            if scenario.config.dishwasher.is_none() {
                scenario.dishwasher = None;
            }
            if cold_start {
                scenario.warm_up_days = 0;
            }
            if perfect_foresight {
                scenario.weather = scenario.weather.with_perfect_forecast();
            }
            scenario.per_asset_weights = !uniform_weights;
            if let Some(price) = co2_eur_per_kg {
                scenario.objective = scenario.objective.with_carbon_price(price);
            }
            if let Some(premium) = autarky_eur_per_kwh {
                scenario.objective = scenario.objective.with_autarky_premium(premium);
            }
            if sharing {
                // Three roofs' worth of neighbours on the same street: the
                // household's own array times three, an equal third of the key.
                scenario.community = Some(hemsd::CommunityMembership::mehrfamilienhaus(
                    scenario
                        .config
                        .pv
                        .map_or(ems_core::prelude::Power::ZERO, |pv| pv.kwp)
                        * 3.0,
                ));
                // And on a day the rule reaches. § 42c Abs. 4 Nr. 1 obliges a
                // network operator to make sharing possible from 1 June 2026,
                // and every reference day is dated before that — so the same
                // day moves forward in whole weeks, keeping its weekday and its
                // season, rather than demonstrating an allocation nobody would
                // perform (D179).
                let before = scenario.date;
                scenario = scenario.on_a_day_sharing_reaches();
                if scenario.date != before {
                    eprintln!(
                        "  § 42c reaches this household from {}, so the same day runs on {}",
                        ems_grid::sharing::SHARING_START,
                        scenario.date
                    );
                }
            }
            scenario.risk = risk.model();
            scenario.adaptive_risk = risk.adaptive();
            let result = hemsd::run(&scenario)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", hemsd::render::day(&scenario, &result));
            }
            // Written **before** the report goes out. The order is the whole
            // point: the household's own record must not depend on a fleet
            // endpoint being up, and the day a network operator asks about is
            // exactly the day the link was down.
            if let Some(path) = &store {
                record_day(path, &result, time::OffsetDateTime::now_utc())?;
            }
            if let Some(url) = report_to {
                let secret = report_secret.ok_or_else(|| {
                    anyhow::anyhow!(
                        "--report-to needs --report-secret (or EMS_OBSD_SECRET): \
                         the fleet will not take an unsigned day"
                    )
                })?;
                report_day(
                    &url,
                    // `simulate` loads no configuration file, so the shell's
                    // defaults are the only trust decision available here: the
                    // platform's own store. A box on a wall reads its own
                    // `[service.http]` and may pin instead.
                    &ems_service::HttpSettings::default(),
                    &ems_events::webhook::parse_secret(&secret)?,
                    &site,
                    &result.kpis(&site, scenario.date),
                    store.as_deref(),
                    time::OffsetDateTime::now_utc(),
                )
                .await?;
            }
        }
        Command::Backtest { day, days, json } => {
            let mut scenario = scenario_for(day, HouseholdConfig::default());
            scenario.per_asset_weights = false;
            let spread = hemsd::spread_over_days(&scenario, days)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "day": scenario.date.to_string(),
                        "days": spread.days(),
                        "pv": {
                            "coverage": spread.pv_forecast.coverage,
                            "crps": spread.pv_forecast.crps,
                            "bias": spread.pv_forecast.bias,
                            "samples": spread.pv_forecast.samples,
                            "skipped": spread.pv_forecast.skipped,
                        },
                        "load": {
                            "coverage": spread.load_forecast.coverage,
                            "crps": spread.load_forecast.crps,
                            "bias": spread.load_forecast.bias,
                            "samples": spread.load_forecast.samples,
                            "skipped": spread.load_forecast.skipped,
                        },
                        "well_calibrated": spread.is_well_calibrated(),
                    }))?
                );
            } else {
                print_backtest(&scenario, &spread);
            }
        }
        Command::Risk { day, days, json } => {
            let config = HouseholdConfig::default();
            let base = scenario_for(day, config);
            let policies = [
                ("one median", Risk::Median),
                ("three futures", Risk::Expected),
                ("…and the tail", Risk::Hedged),
                ("only when at risk", Risk::Adaptive),
            ];
            let mut rows = Vec::new();
            for (label, policy) in policies {
                let mut scenario = base.clone();
                scenario.risk = policy.model();
                scenario.adaptive_risk = policy.adaptive();
                // The per-asset weights change no reference-day outcome and cost
                // a second solve of the same model; the sweep is about the
                // *plan*, so it runs without them and takes a third of the time.
                scenario.per_asset_weights = false;
                rows.push((label, hemsd::spread_over_days(&scenario, days)?));
            }
            if json {
                let as_json: Vec<_> = rows
                    .iter()
                    .map(|(label, s)| {
                        serde_json::json!({
                            "policy": label,
                            "days": s.days(),
                            "mean_saving_eur": s.mean_saving_eur(),
                            "worst_saving_eur": s.worst_saving_eur(),
                            "best_saving_eur": s.best_saving_eur(),
                            "mean_unserved_eur": s.mean_unserved_eur(),
                            "worst_unserved_eur": s.worst_unserved_eur(),
                            "seconds": s.seconds,
                            "pv_coverage": s.pv_forecast.coverage,
                            "pv_crps": s.pv_forecast.crps,
                            "load_coverage": s.load_forecast.coverage,
                            "load_crps": s.load_forecast.crps,
                            "well_calibrated": s.is_well_calibrated(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&as_json)?);
            } else {
                print_risk(&base, &rows);
            }
        }
    }
    Ok(())
}

/// Keep the day's evidence and registers in the box's own store.
///
/// Written **before** anything is sent anywhere, and nothing is marked forwarded
/// here: until a fleet client drains the outbox it grows, which is a number the
/// box can report about itself.
fn record_day(
    path: &std::path::Path,
    result: &hemsd::DayResult,
    now: time::OffsetDateTime,
) -> anyhow::Result<()> {
    let mut store = hemsd::store::Store::open(path)?;
    let recorded: Vec<hemsd::store::Recorded> = result
        .quarter_hours
        .iter()
        .map(|q| hemsd::store::Recorded {
            registers: *q,
            production: result.production_kwh_by_slot.get(&q.slot).copied(),
            watched: false,
            replay: None,
            storage: None,
            terms: None,
        })
        .collect();
    store.put_quarter_hours(&recorded, now)?;
    for event in &result.evidence {
        store.put_control_event(event)?;
    }
    let backlog = store.backlog()?;
    eprintln!(
        "\n  kept in {}\n  …waiting for the fleet      {} events, {} quarter hours",
        path.display(),
        backlog.events,
        backlog.quarter_hours
    );
    Ok(())
}

/// Send one day's KPIs to an `obsd`, as a signed CloudEvent.
///
/// A failure is **reported and not fatal** — a box whose fleet endpoint is down
/// has still managed its household correctly, and exiting non-zero would say
/// otherwise.
///
/// # A failed send is a queued day, where there is somewhere to queue it
///
/// With a `--store` the day is written to the outbox **before** the attempt, so
/// a fleet that is down costs a delay rather than a day; `hemsd run`'s drain
/// picks it up. Without one there is nowhere to put it, and the failure is the
/// end of that day — which is why `--store` is what a household runs with.
///
/// # Why the message id is the site and the day
///
/// `obsd` is idempotent by date: a box that comes back after an outage and
/// re-sends yesterday is correcting itself rather than adding a day. A random
/// identifier would make the *same* correction a different message every time,
/// so the identifier is derived from what the report is about, and the receiver
/// de-duplicates on the same string the signature covers.
async fn report_day(
    url: &str,
    http: &ems_service::HttpSettings,
    secret: &ems_events::webhook::Webhook,
    site: &str,
    kpis: &ems_core::report::DayKpis,
    store: Option<&std::path::Path>,
    now: time::OffsetDateTime,
) -> anyhow::Result<()> {
    let event = ems_events::Event::new(
        ems_events::SITE_DAY_REPORTED,
        format!("ems://sites/{site}"),
        format!("{site}:{}", kpis.date),
        now,
        kpis,
    )
    .about(kpis.date.to_string());
    let body = event.to_bytes()?;
    let endpoint = format!("{}/v1/days", url.trim_end_matches('/'));
    // Checked before the day is built into a request, and it is the one failure
    // here that is **fatal**: a fleet endpoint that is down costs a dashboard,
    // and one that is plaintext costs the household's privacy every day until
    // somebody notices.
    hemsd::report::is_confidential(&endpoint)?;

    // Queued first. The order is the same one the evidence record uses and for
    // the same reason: what is written down survives the send failing, and what
    // is only in flight does not.
    let queued = match store {
        Some(path) => {
            let mut store = hemsd::store::Store::open(path)?;
            Some(store.queue_event(&event.id, ems_events::SITE_DAY_REPORTED, &body, now)?)
        }
        None => None,
    };

    let signature = ems_events::webhook::sign(secret, &event.id, now, &body)?;
    match hemsd::report::post_event(&endpoint, http, body, &signature.headers()).await {
        Ok(status) if (200..300).contains(&status) => {
            eprintln!("\n  reported to {endpoint} — HTTP {status}");
            if let (Some(path), Some(id)) = (store, queued) {
                // Taken, so out of the backlog — by the row identifier the queue
                // handed back rather than by a search through it, which on a box
                // with a real backlog would not find the row it had just added.
                hemsd::store::Store::open(path)?.mark_sent(&[id], now)?;
            }
        }
        Ok(status) => {
            eprintln!("\n  {endpoint} answered HTTP {status}");
            eprintln!("  the day is queued and `hemsd run` will try again");
        }
        Err(e) => {
            eprintln!("\n  could not report to {endpoint}: {e}");
            if store.is_some() {
                eprintln!("  the day is queued and `hemsd run` will try again");
            }
        }
    }
    Ok(())
}

/// `HH:MM` as a duration since midnight.
#[cfg(feature = "house")]
fn time_of_day(text: &str) -> Result<time::Duration, String> {
    let parsed = text.split_once(':').and_then(|(h, m)| {
        let (h, m) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?);
        ((0..24).contains(&h) && (0..60).contains(&m)).then(|| time::Duration::minutes(h * 60 + m))
    });
    parsed.ok_or_else(|| format!("`{text}` is not a time of day like 16:58"))
}

/// `hemsd house`'s arguments, as the command line gave them.
#[cfg(feature = "house")]
struct HouseArgs {
    day: Day,
    at: Option<time::Duration>,
    listen: std::net::SocketAddr,
    ship: std::net::SocketAddr,
    api: std::net::SocketAddr,
    out: PathBuf,
    keep_history: bool,
    exposed: bool,
    manage_storage: bool,
    with_tank: bool,
    with_room: bool,
    with_ev: bool,
    run: bool,
}

/// The virtual house: start it, write the box's configuration, and report what
/// the devices are doing until interrupted — managed by a box in the same
/// process when `run` is set.
#[cfg(feature = "house")]
async fn house(args: HouseArgs) -> anyhow::Result<()> {
    use hemsd::house;
    let HouseArgs {
        day,
        at,
        listen,
        ship,
        api,
        out,
        keep_history,
        exposed,
        run,
        manage_storage,
        with_tank,
        with_room,
        with_ev,
    } = args;
    anyhow::ensure!(
        !manage_storage || listen.ip().is_loopback(),
        "managed storage requires loopback"
    );
    let out = out.as_path();

    let scenario = scenario_for(day, HouseholdConfig::default());
    let house = house::start(house::Options {
        scenario: scenario.clone(),
        listen,
        household: Some(ship),
        at,
        identity: Some(out.with_extension("steuerbox.pem")),
        appliances: house::Appliances {
            tank: with_tank,
            room: with_room,
            ev: with_ev,
        },
        exposed,
    })
    .await?;
    let mut config = String::new();
    if keep_history {
        let store = std::path::absolute(out)?.with_added_extension("redb");
        config.push_str(&toml::to_string(&std::collections::BTreeMap::from([(
            "store_path",
            &store,
        )]))?);
        println!("  history    {}", store.display());
    }
    config.push_str(&house::configuration(
        &house,
        &scenario,
        Some(ship),
        api,
        manage_storage,
    )?);
    std::fs::write(out, config)?;

    let minutes = house.clock.since_midnight().whole_minutes();
    println!(
        "  a virtual house on {} at {:02}:{:02}",
        scenario.date,
        minutes / 60,
        minutes % 60
    );
    println!(
        "  modbus     {}  — inverter (unit 1), meter (2), battery (3)",
        house.modbus
    );
    if with_ev {
        println!("  wallbox    current limit, vehicle charge and outlet Wh (unit 6)");
    }
    if with_room {
        println!("  room heat  indoor temperature, power and operating state (unit 5)");
    }
    if with_tank {
        println!("  hot water  relay and measured tank temperature (unit 4)");
    }
    if let Some(ski) = &house.steuerbox_ski {
        println!("  steuerbox  SKI {ski}, dialling {ship}");
    }
    if let Some((from, until, limit)) = scenario.grid_event {
        println!(
            "  § 14a      {:.1} kW from {:02}:{:02} to {:02}:{:02}",
            limit.kw(),
            from.whole_hours(),
            from.whole_minutes() % 60,
            until.whole_hours(),
            until.whole_minutes() % 60,
        );
    }
    if run {
        println!(
            "\n  the box is starting; open http://{api} and sign in with the code it prints\n"
        );
    } else {
        println!("\n  now, in a second terminal:\n");
        println!("      hemsd run --config {}\n", out.display());
        println!("  and open http://{api}\n");
    }

    // SIGTERM as well as Ctrl-C: a house under a service manager or in a
    // container is stopped the way every other daemon here is.
    let (stop, trigger) = ems_service::Shutdown::channel();
    tokio::spawn(ems_service::shutdown::on_signal(trigger));
    let report = async {
        let mut every = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            every.tick().await;
            let r = house.reading();
            let seen = *house.steuerbox.borrow();
            let minutes = house.clock.since_midnight().whole_minutes();
            println!(
                "  {:02}:{:02}  roof {:>5.0} W  house {:>5.0} W  battery {:>+6.0} W ({:>3.0} %)  grid {:>+6.0} W  steuerbox {}",
                (minutes / 60) % 24,
                minutes % 60,
                r.pv_w,
                r.load_w,
                r.battery_w,
                r.soc * 100.0,
                r.grid_w,
                match (seen.connected, seen.accepted_w) {
                    (false, _) => "waiting for the box".to_owned(),
                    (true, None) => "connected, no limit".to_owned(),
                    (true, Some(w)) => format!("limit {:.1} kW accepted", w / 1000.0),
                },
            );
        }
    };
    // The box, managing the house from the file the house just wrote — the
    // same path `hemsd run` takes, in the same process, and raced rather than
    // spawned: a box that stops ends the command instead of leaving a house
    // reporting to nobody.
    let config = out.to_path_buf();
    let managed = async {
        if run {
            manage(Some(&config), false, None).await
        } else {
            std::future::pending().await
        }
    };
    tokio::select! {
        () = report => Ok(()),
        () = stop.clone().wait() => Ok(()),
        result = managed => result.map_err(|e| anyhow::anyhow!("the box stopped: {e:#}")),
    }
}

fn scenario_for(day: Day, config: HouseholdConfig) -> Scenario {
    match day {
        Day::Winter => Scenario::winter_with_grid_event(config),
        Day::Summer => Scenario::summer_surplus(config),
        Day::Deadline => Scenario::winter_evening_deadline(config),
        Day::Shared => Scenario::winter_evening_no_store(&config),
        Day::Offline => Scenario::summer_without_a_planner(config),
        Day::Autumn => Scenario::autumn_without_a_planner(config),
        Day::Capped => Scenario::summer_capped(&config),
    }
}

fn print_backtest(scenario: &Scenario, s: &hemsd::Spread) {
    println!(
        "\n  {} — {} weathers, the band scored against every one of them\n",
        scenario.date,
        s.days()
    );
    println!(
        "  {:<16}{:>10}{:>10}{:>9}{:>10}{:>9}",
        "band", "covered", "CRPS", "bias", "scored", "dark"
    );
    for (label, c) in [
        ("production", &s.pv_forecast),
        ("household load", &s.load_forecast),
    ] {
        println!(
            "  {:<16}{:>9.0}%{:>9.0}W{:>8.0}W{:>10}{:>9}",
            label,
            c.coverage * 100.0,
            c.crps,
            c.bias,
            c.samples,
            c.skipped
        );
    }
    println!(
        "\n  a 10–90 band should cover 80 %; `dark` is the quarter hours where\n  \
         there was nothing to forecast and which are therefore not scored.\n"
    );
    println!(
        "  {}\n",
        if s.is_well_calibrated() {
            "calibrated — over enough independent days to say so."
        } else if s.pv_forecast.episodes < ems_forecast::CALIBRATION_DAYS {
            "too few days to call it either way — try --days 20."
        } else {
            "not calibrated: the plan is hedging against a future of the wrong width."
        }
    );
}

fn print_risk(scenario: &Scenario, rows: &[(&str, hemsd::Spread)]) {
    let days = rows.first().map_or(0, |(_, s)| s.days());
    println!(
        "\n  {} — {days} weathers, the same household under each policy\n",
        scenario.date
    );
    println!(
        "  {:<16}{:>10}{:>10}{:>10}{:>12}{:>10}",
        "policy", "mean", "worst", "best", "unserved", "solve"
    );
    for (label, s) in rows {
        println!(
            "  {:<16}{:>9.2}€{:>9.2}€{:>9.2}€{:>11.2}€{:>9.0}s",
            label,
            s.mean_saving_eur(),
            s.worst_saving_eur(),
            s.best_saving_eur(),
            s.mean_unserved_eur(),
            s.seconds
        );
    }
    // The sweep is also the only thing in this workspace that produces
    // *independent* days, so it is the only thing that can say whether the band
    // the whole hedge is planned against is the width it claims to be. Read off
    // the first policy, because the forecasts do not depend on the policy — the
    // box learned the same roof either way.
    if let Some((_, s)) = rows.first() {
        let pv = &s.pv_forecast;
        let load = &s.load_forecast;
        println!(
            "\n  {:<16}{:>10}{:>10}{:>12}",
            "band", "covered", "CRPS", "episodes"
        );
        for (label, c, unit) in [("production", pv, "W"), ("household load", load, "W")] {
            println!(
                "  {:<16}{:>9.0}%{:>9.0}{unit}{:>12}",
                label,
                c.coverage * 100.0,
                c.crps,
                c.episodes
            );
        }
        println!(
            "\n  a 10–90 band should cover 80 %. {}",
            if s.is_well_calibrated() {
                "It does, over enough days to say so."
            } else if pv.episodes < ems_forecast::CALIBRATION_DAYS {
                "Too few days to say — run --days 20."
            } else {
                "It does not: the plan is hedging against a future that is the \
                 wrong width."
            }
        );
    }
    println!(
        "\n  mean is what the household saves on an average day; worst is the day\n  \
         it bought the insurance for. A hedge worth having has a lower mean and a\n  \
         higher worst — and if it has a lower mean and nothing else, it is not\n  \
         worth having, which is a result rather than a failure.\n"
    );
}
