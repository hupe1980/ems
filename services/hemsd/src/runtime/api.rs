//! What the box will tell you about itself.
//!
//! Small on purpose. The fleet's view of a household is `obsd`'s and the
//! household's own history is `histd`'s; what a box needs locally is the answer
//! to "what is it doing, and why" — which is the one question neither of those
//! can answer while the WAN is down, and the one an installer standing next to
//! the box is actually asking.
//!
//! # One write, and it is a *desire* rather than a setpoint
//!
//! No endpoint here commands a device. A setpoint that did not come through the
//! arbiter would not have been through the guard, which is the one property this
//! whole workspace is built to keep: `[BK6-22-300 A1 4.6 S. 3]` makes a network
//! operator's reduction win over market-driven control, and an HTTP handler that
//! could write a value straight to a driver would be a second control plane
//! nobody audited.
//!
//! `/v1/overrides` is therefore the only write, and it is safe for exactly that
//! reason: what it changes is what the arbiter *wants*, which the guard then
//! narrows like anything else. A household in the middle of a § 14a reduction
//! that presses boost gets as much as the reduction allows and not a watt more.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::{get, put};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::runtime::control::Status;
use crate::runtime::ship::Trust;
use time::OffsetDateTime;

/// What the API needs to answer.
#[derive(Clone)]
pub struct Local {
    heat_meter: Option<super::heat_meter::Observer>,
    #[cfg(feature = "matter")]
    matter: Option<super::matter::Meter>,
    status: Arc<Mutex<Status>>,
    site: String,
    ski: Option<String>,
    overrides: crate::runtime::overrides::Overrides,
    /// Approving a Steuerbox without restarting the box.
    ///
    /// `None` where this household has no EEBUS identity at all, which is a box
    /// with no § 14a driver and nothing to pair.
    trust: Option<crate::runtime::ship::Trust>,
    /// The credentials this box answers to, so a household can see which energy
    /// managers it has connected and withdraw one.
    access: crate::runtime::access::LocalAccess,
    /// The box's own measurement series, where it keeps one.
    ///
    /// Read-only here. The Data Act gives a user the data their product
    /// generates, and on the box that means the household's own local API rather
    /// than a fleet service — this is the one-second half of it.
    series: Option<Arc<crate::series::Series>>,
    history_reader: Arc<tokio::sync::Semaphore>,
    /// What only the household page reads: the site, the plan, the goal.
    home: HomeSources,
    /// What the configuration says about this box's devices, for the handover
    /// sheet and discovery.
    commissioning: crate::runtime::handover::Commissioning,
    settings: Option<Arc<crate::Settings>>,
    installer: Option<(
        Arc<super::installation::Installer>,
        ems_service::shutdown::ShutdownTrigger,
    )>,
    registry: Option<super::transport::Shared>,
    /// The store the closed days are read from, where the box keeps one.
    days: Option<Arc<Mutex<crate::store::Store>>>,
    /// The Web Push sender, where the box has a store to keep subscriptions in.
    push: Option<crate::runtime::push::Push>,
    /// Events out: the systems the household asked the box to tell.
    webhooks: Option<crate::runtime::webhooks::Webhooks>,
}

/// What the household page needs beyond `/v1/status`.
#[derive(Clone, Default)]
pub struct HomeSources {
    /// Planner readiness shared with the control loop.
    pub health: ems_service::Health,
    /// The house, so the page can name its devices.
    pub site: Option<Arc<ems_core::prelude::Site>>,
    /// The plan in force.
    pub plan: Arc<tokio::sync::RwLock<Option<ems_core::prelude::Plan>>>,
    /// The prices it was made against.
    pub prices: Arc<tokio::sync::RwLock<Option<ems_tariff::PriceStack>>>,
    /// The household's goals, shared with the planner.
    pub goal: crate::home::Goals,
    /// Trigger a fresh plan after a contract change.
    pub tariff_changed: Arc<tokio::sync::Notify>,
    /// The car as the planner last saw it.
    pub car: crate::home::CarFacts,
    /// The tank as the planner last saw it.
    pub tank: crate::home::TankFacts,
    /// The planner's last solve, for pricing a goal before it is set.
    pub last: crate::runtime::planner::LastInputs,
    /// The SHA-256 fingerprint of the box's SHIP certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate it serves HTTPS with.
    pub https_fingerprint: Option<String>,
}

impl Local {
    /// Attach the live wired heat meter to installer diagnostics.
    #[must_use]
    pub fn with_heat_meter(mut self, observer: super::heat_meter::Observer) -> Self {
        self.heat_meter = Some(observer);
        self
    }

    /// Attach the diagnostic appliance reading to the household page.
    #[cfg(feature = "matter")]
    #[must_use]
    pub fn with_matter(mut self, meter: super::matter::Meter) -> Self {
        self.matter = Some(meter);
        self
    }

    /// Give the household page what it reads beyond `/v1/status`.
    #[must_use]
    pub fn with_home(mut self, home: HomeSources) -> Self {
        self.home = home;
        self
    }

    /// Say what the configuration names, so the handover sheet and discovery
    /// can set what they find against it.
    #[must_use]
    pub fn with_commissioning(
        mut self,
        commissioning: crate::runtime::handover::Commissioning,
    ) -> Self {
        self.commissioning = commissioning;
        self
    }

    /// Supply the effective site configuration for read-only installer previews.
    #[must_use]
    pub fn with_settings(mut self, settings: crate::Settings) -> Self {
        self.settings = Some(Arc::new(settings));
        self
    }

    /// Read commissioning diagnostics from the active protocol actors.
    #[must_use]
    pub fn with_registry(mut self, registry: super::transport::Shared) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Expose an aggregate control-box alert without household or device identifiers.
    ///
    /// # Errors
    /// Returns the registry error if the gauge cannot be registered.
    pub fn publish_control_box_health(
        &self,
    ) -> anyhow::Result<Option<ems_service::metrics::RegisteredGauge>> {
        let (Some(asset), Some(trust)) =
            (self.commissioning.control_box.clone(), self.trust.clone())
        else {
            return Ok(None);
        };
        let status = Arc::clone(&self.status);
        let gauge = ems_service::metrics::publish_gauge(
            "hems_control_box_needs_attention",
            "1 when the configured control box is unpaired, silent or in failsafe, 0 when heard",
            Box::new(move || {
                let Ok(status) = status.try_lock() else {
                    return f64::NAN;
                };
                let state = control_box_state(Some(&asset), Some(&trust), &status);
                f64::from(state.state != "heard")
            }),
        )?;
        Ok(Some(gauge))
    }

    /// Enable reviewed installation changes on a supervised, file-backed gateway.
    #[must_use]
    pub fn with_installer(
        mut self,
        installer: Arc<super::installation::Installer>,
        restart: ems_service::shutdown::ShutdownTrigger,
    ) -> Self {
        self.installer = Some((installer, restart));
        self
    }

    /// Answer `/v1/push` with this sender.
    #[must_use]
    pub fn with_push(mut self, push: Option<crate::runtime::push::Push>) -> Self {
        self.push = push;
        self
    }

    /// The webhook outlet: `/v1/subscriptions` and `/v1/deliveries`.
    #[must_use]
    pub fn with_webhooks(mut self, webhooks: Option<crate::runtime::webhooks::Webhooks>) -> Self {
        self.webhooks = webhooks;
        self
    }

    /// The household page's document as of now — `None` on a box that has not
    /// been given a site to describe.
    ///
    /// What `GET /v1/home` answers, and what the push watcher reads: one
    /// function, so a notification cannot say what the page does not.
    pub async fn document(&self) -> Option<crate::home::Home> {
        let site = self.home.site.as_ref()?;
        let status = self.status.lock().await;
        let plan = self.home.plan.read().await;
        let prices = self.home.prices.read().await;
        let goals = self.home.goal.get();
        let car = *self.home.car.lock().await;
        let tank = *self.home.tank.lock().await;
        let mut document = crate::home::live(
            &crate::home::Live {
                site,
                status: &status,
                plan: plan.as_ref(),
                prices: prices.as_ref(),
                goals,
                car,
                tank,
                ski: self.ski.clone(),
                fingerprint: self.home.fingerprint.clone(),
                https_fingerprint: self.home.https_fingerprint.clone(),
            },
            OffsetDateTime::now_utc(),
        );
        if self
            .home
            .health
            .readiness()
            .probes
            .get("planner")
            .is_some_and(|probe| !probe.ready)
        {
            document.previous_plan_at = plan.as_ref().map(|p| rfc3339(p.created_at));
        }
        #[cfg(feature = "matter")]
        if let Some(meter) = &self.matter {
            document.matter = Some(meter.view());
        }
        document.charging_now = self.home.goal.boosting(OffsetDateTime::now_utc());
        Some(document)
    }

    /// Answer `/v1/days` from the closed days `store` keeps.
    #[must_use]
    pub fn with_days(mut self, store: Option<Arc<Mutex<crate::store::Store>>>) -> Self {
        self.days = store;
        self
    }

    /// The surface over this box's own status.
    #[must_use]
    pub fn new(
        status: Arc<Mutex<Status>>,
        site: String,
        ski: Option<String>,
        overrides: crate::runtime::overrides::Overrides,
        trust: Option<crate::runtime::ship::Trust>,
        access: crate::runtime::access::LocalAccess,
        series: Option<Arc<crate::series::Series>>,
    ) -> Self {
        Self {
            status,
            site,
            ski,
            overrides,
            trust,
            access,
            series,
            history_reader: Arc::new(tokio::sync::Semaphore::new(1)),
            #[cfg(feature = "matter")]
            matter: None,
            home: HomeSources::default(),
            commissioning: crate::runtime::handover::Commissioning::default(),
            settings: None,
            installer: None,
            registry: None,
            heat_meter: None,
            days: None,
            push: None,
            webhooks: None,
        }
    }
}

/// One tick, as JSON.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StatusBody {
    /// Which household.
    pub site: String,
    /// The box's EEBUS Subject Key Identifier, where it has one.
    ///
    /// What an installer gives the metering point operator so a Steuerbox can be
    /// told to trust this box. Field reports make that exchange the single most
    /// common § 14a commissioning failure there is, which is why it is on the
    /// first page a box will show rather than in a log.
    pub ski: Option<String>,
    /// When the last tick ran, RFC 3339.
    pub at: Option<String>,
    /// What every asset was last told, in watts.
    pub commanded_w: std::collections::BTreeMap<String, f64>,
    /// The assets no driver has been heard from.
    pub silent: Vec<String>,
    /// Devices whose available power is a nameplate rather than a reading.
    ///
    /// Worth a field of its own: a curtailed inverter that cannot say what it
    /// *could* produce is one whose curtailment lifts on an assumption, and a
    /// household is entitled to know which of its devices are in that position.
    pub assumed_available: Vec<String>,
    /// Controllable devices whose **consumption** the guard had to assume.
    ///
    /// The consumption side of the same honesty: a silent controllable device is
    /// taken to be drawing its nameplate power, which is the safe answer and an
    /// expensive one — every watt of it is § 14a budget spent on a device that
    /// may be doing nothing. `assumed_available` is the generation side (R20).
    pub assumed_nominal: Vec<String>,
    /// Devices that answered their last setpoint and did not act on it, with
    /// what they said about it.
    ///
    /// The one fault the box is otherwise blind to. A driver that reports a
    /// refusal is easy to see; a device that acknowledges the write, stores the
    /// setpoint and never switches it on answers exactly like one that obeyed,
    /// and the disagreement only surfaces as a meter that will not match the
    /// plan. Separate from `silent`, because a silent device is a network fault
    /// and this one is a device that has to be commanded some other way.
    pub disobedient: std::collections::BTreeMap<String, String>,
    /// Controllable devices no driver speaks for.
    ///
    /// The first thing to look at when a device is not doing what this page says
    /// it was told to: the setpoint was decided and had nowhere to go.
    pub undriven: Vec<String>,
    /// Whether the roof is still delivering what it used to.
    ///
    /// The performance ratio of IEC 61724-1 against this array's own seasonal
    /// baseline, and a verdict. It is on the status page rather than in a log
    /// because it is the one fault a household can *act* on — the others here
    /// are for an installer — and because the box is otherwise the last thing
    /// that would notice: its own forecast corrector is built to learn a lower
    /// roof and carry on planning well (D199).
    #[schemars(schema_with = "crate::openapi::roof_health")]
    pub roof_health: Option<ems_forecast::Health>,
    /// The § 14a ceiling in force, kW.
    pub steuve_ceiling_kw: Option<f64>,
    /// What the controllable devices may draw in total, surplus included, kW.
    pub steuve_budget_kw: Option<f64>,
    /// The netzwirksamer Leistungsbezug right now, kW — `[A1 2.3]`.
    pub netzwirksam_kw: Option<f64>,
    /// How far the grid meter is from the sum of the assets, W.
    pub balance_residual_w: Option<f64>,
    /// How old the plan in force is, minutes — `null` where the box has never
    /// published one.
    ///
    /// The seam a box is most likely to be quietly broken at: one that never
    /// plans looks exactly like one that plans badly, and the difference is a
    /// whole tariff's worth of money.
    pub minutes_without_a_plan: Option<i64>,
    /// What the plan in force expects its horizon to cost, euros — wear,
    /// discomfort and undelivered service included.
    pub plan_expected_eur: Option<f64>,
    /// How many control ticks took longer than a control period.
    pub overruns: u64,
    /// How exposed this connection is to § 14a control — `null` until the box
    /// has any record to draw on.
    pub exposure: Option<ExposureBody>,
    /// Which rule is holding each asset's setpoint, where one is.
    ///
    /// The answer to *why is my car charging slowly*, which is the commonest
    /// question a household has and the one a screen of watts cannot answer. It
    /// is the same reason chain a setpoint carries (P7), summarised per asset so
    /// a page does not have to reconstruct it.
    pub overriding: std::collections::BTreeMap<String, String>,
    /// What an external Customer Energy Manager is doing over S2, where one is
    /// connected.
    ///
    /// A household that has delegated its optimising to somebody else is
    /// entitled to see that it has, and to see which of its devices that manager
    /// is actually driving — an S2 session that connected, chose a control type
    /// and then instructed nothing looks from every other screen exactly like
    /// one that is working.
    pub cem: crate::runtime::s2::CemStatus,
}

/// How often the operator reduces *this* household, and when.
///
/// The question `[A1 8.4]` cannot answer. What the operator publishes is a
/// monthly aggregate over a whole Netzbereich with no timestamps in it, so the
/// only record of when this connection was reduced is the one the box keeps for
/// `[A1 7.2]` anyway. Reported, and deliberately not planned against (D129).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ExposureBody {
    /// How many days of record the figures are drawn from.
    ///
    /// The denominator, on the page. A share whose denominator is invisible
    /// cannot be checked, which is R28.
    pub days_of_record: i64,
    /// Hours the household was reduced for, over that record.
    pub hours_reduced: f64,
    /// The quarter hour of the local day it happens in most often, as `HH:MM` —
    /// `null` where it has never happened.
    pub busiest_quarter_hour: Option<String>,
    /// How often a reduction is in force in that quarter hour, in `[0, 1]`.
    pub busiest_frequency: f64,
    /// The ceiling typically commanded there, kW.
    pub typical_ceiling_kw: Option<f64>,
}

/// The routes this daemon adds to the shared health surface.
pub fn router(local: Local) -> axum::Router {
    axum::Router::new()
        .route("/v1/status", get(status))
        .route("/v1/pairing", get(list_trusted).post(approve_peer))
        .route("/v1/pairing/{ski}", axum::routing::delete(forget_peer))
        .route("/v1/pairing/{ski}/refuse", axum::routing::post(refuse_peer))
        .route("/v1/overrides", get(list_overrides).delete(clear_overrides))
        .route("/v1/series/{point}", get(series))
        .route("/v1/managers", get(list_managers))
        .route(
            "/v1/managers/{name}",
            put(connect_manager).delete(forget_manager),
        )
        .route(
            "/v1/overrides/{asset}",
            put(set_override).delete(clear_override),
        )
        .route("/v1/home", get(home))
        .route("/v1/tariff", get(tariff_status).put(activate_tariff))
        .route("/v1/tariff/preview", axum::routing::post(preview_tariff))
        .route("/v1/qr/{what}", get(qr))
        .route("/v1/handover", get(handover))
        .route("/v1/commissioning", get(commissioning))
        .route(
            "/v1/commissioning/transfer",
            get(control_transfer).put(review_transfer),
        )
        .route("/v1/discover", axum::routing::post(discover))
        .route(
            "/v1/commissioning/change",
            get(installation_outcome).post(apply_installation),
        )
        .route("/v1/trust", get(trust_check))
        .route("/v1/token", axum::routing::post(replace_token))
        .route("/v1/days", get(days))
        .route("/v1/days.csv", get(days_csv))
        .route("/v1/goals/{asset}", put(set_goal).delete(clear_goal))
        .route("/v1/goals/{asset}/quote", axum::routing::post(quote_goal))
        .route(
            "/v1/goals/{asset}/boost",
            put(start_boost).delete(cancel_boost),
        )
        .route(
            "/v1/push",
            get(push_overview).post(push_subscribe).delete(push_forget),
        )
        .route("/v1/push/test", axum::routing::post(push_test))
        .route("/v1/subscriptions", get(list_subscriptions).post(subscribe))
        .route(
            "/v1/subscriptions/{id}/revoke",
            axum::routing::post(revoke_subscription),
        )
        .route(
            "/v1/subscriptions/{id}/test",
            axum::routing::post(test_subscription),
        )
        .route("/v1/deliveries", get(deliveries))
        // An unknown path under `/v1/` is a refusal like any other, so it passes
        // the same layers: the router's own fallback would not.
        .route("/v1/{*rest}", axum::routing::any(no_such_route))
        .with_state(local)
}

type SheetError = (axum::http::StatusCode, String);

async fn tariff_status(
    State(local): State<Local>,
) -> Result<Json<crate::price_sheet::Status>, SheetError> {
    use crate::price_sheet::Schedule;
    let slot = ems_core::prelude::Slot::containing(time::OffsetDateTime::now_utc());
    let schedule = match &local.days {
        Some(store) => Schedule::load(&*store.lock().await).map_err(|_| {
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "cannot read signed tariff schedule".into(),
            )
        })?,
        None => Schedule::default(),
    };
    let prices = local.home.prices.read().await;
    let fallback = local.settings.as_ref().and_then(|settings| {
        rust_decimal::Decimal::try_from(settings.tariff.fallback_ct_per_kwh).ok()
    });
    Ok(Json(schedule.status(
        slot,
        prices.as_ref(),
        fallback,
        local.days.is_some(),
    )))
}

async fn preview_tariff(
    State(local): State<Local>,
    Json(request): Json<crate::price_sheet::Import>,
) -> Result<Json<crate::price_sheet::Review>, SheetError> {
    review_tariff(local, request, false).await
}

async fn activate_tariff(
    State(local): State<Local>,
    Json(request): Json<crate::price_sheet::Import>,
) -> Result<Json<crate::price_sheet::Review>, SheetError> {
    review_tariff(local, request, true).await
}

async fn review_tariff(
    local: Local,
    request: crate::price_sheet::Import,
    activate: bool,
) -> Result<Json<crate::price_sheet::Review>, SheetError> {
    use axum::http::StatusCode;
    let store = local.days.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "tariff approval requires durable storage".into(),
    ))?;
    let signed = crate::price_sheet::verify(request.signed.as_bytes())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let now = ems_core::prelude::Slot::containing(time::OffsetDateTime::now_utc());
    let prices = local.home.prices.read().await.clone();
    let store = store.lock().await;
    let schedule = crate::price_sheet::Schedule::load(&store).map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "cannot read signed tariff schedule".into(),
        )
    })?;
    let review = signed
        .review(&schedule, now, prices.as_ref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if activate {
        if request.confirmation.as_ref() != Some(&review) {
            return Err((
                StatusCode::CONFLICT,
                "review the current signer, price and contract before accepting".into(),
            ));
        }
        store
            .put_price_sheet(
                now.start().unix_timestamp(),
                review.effective_at,
                &request.signed,
            )
            .map_err(|_| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "could not save signed tariff".into(),
                )
            })?;
        local.home.tariff_changed.notify_one();
    }
    Ok(Json(review))
}
async fn no_such_route() -> (axum::http::StatusCode, &'static str) {
    (
        axum::http::StatusCode::NOT_FOUND,
        "there is no such route; `GET /openapi.json` lists them",
    )
}

/// The energy managers this household has connected.
///
/// The § 14a side has had this since D102: a Steuerbox is trusted by SKI, shown
/// on a screen and forgotten from one. A Customer Energy Manager drives the same
/// house through `/s2/{asset}` and, until now, held the household's *own*
/// credential — so a household could see that something was driving its battery
/// and not what, and could withdraw one only by rotating the token every other
/// surface uses (D188).
async fn list_managers(
    State(local): State<Local>,
) -> Result<Json<Vec<crate::runtime::access::Manager>>, StatusCode> {
    local.access.managers().await.map(Json).map_err(|error| {
        tracing::error!(%error, "the manager list could not be read");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Connect one, and hand back the credential it presents.
///
/// **The only time the token is shown.** It is not stored in a form this API can
/// give back, for the same reason the EEBUS private key is not: a credential an
/// endpoint can be asked for is a credential an endpoint can leak. Naming the
/// same manager again issues a new one, which is how a household rotates it.
async fn connect_manager(
    State(local): State<Local>,
    Path(name): Path<String>,
) -> Result<Json<Connected>, (StatusCode, String)> {
    if name.trim().is_empty() || name.len() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            "a manager needs a name a household would recognise, of at most 64 characters".into(),
        ));
    }
    let token = local
        .access
        .connect_manager(&name)
        .await
        .map_err(|e| match e {
            crate::runtime::access::ManagerError::Name(_) => {
                (StatusCode::BAD_REQUEST, e.to_string())
            }
            crate::runtime::access::ManagerError::Store(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;
    Ok(Json(Connected { name, token }))
}

/// Withdraw one. The credential stops working on the next request.
async fn forget_manager(
    State(local): State<Local>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let existed = local
        .access
        .forget_manager(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

/// A manager that has just been connected, and the credential it presents.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct Connected {
    /// What the household called it.
    pub name: String,
    /// The bearer token it presents. Shown once and never again.
    pub token: String,
}

/// The window a series is asked for.
///
/// Named for the asking, not the answer: what comes back is a
/// [`crate::series::Window`], which carries the resolution the box could answer
/// at as well as the readings.
#[derive(Debug, serde::Deserialize)]
pub struct Asked {
    /// The first instant, RFC 3339. Absent means an hour ago, which is what a
    /// screen wants and what stops an unbounded default returning a week.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub from: Option<time::OffsetDateTime>,
    /// The instant after the last, RFC 3339. Absent means now.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub to: Option<time::OffsetDateTime>,
    /// Request summaries for a short window. Omission selects the available tier.
    pub resolution: Option<HistoryResolution>,
}

/// The tier that clients can explicitly request.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryResolution {
    /// Whole quarter-hour buckets.
    QuarterHours,
}

/// One point of measurement over a window — `grid`, `netzwirksam`, or an asset.
///
/// The household's own history, from the box that took it. A box with no series
/// configured answers `404` rather than an empty list: *nothing was kept* and
/// *nothing happened* are different answers, and an empty array would say the
/// second.
///
/// The answer carries its own `resolution`, because the box keeps days of
/// seconds and years of quarter hours and a long window is answered from the
/// second tier. A client that ignored it would draw a year of quarter-hourly
/// means and label it one-second data.
async fn series(
    State(local): State<Local>,
    Path(point): Path<String>,
    Query(window): Query<Asked>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse as _;
    let store = local.series.ok_or(StatusCode::NOT_FOUND)?;
    let to = window.to.unwrap_or_else(|| {
        let now = time::OffsetDateTime::now_utc();
        if window.resolution.is_some() {
            ems_core::prelude::Slot::containing(now).start()
        } else {
            now
        }
    });
    let from = window
        .from
        .or_else(|| to.checked_sub(time::Duration::HOUR))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let span = to - from;
    if !span.is_positive()
        || span > time::Duration::days(1096)
        || [from, to]
            .iter()
            .any(|t| i64::try_from(t.unix_timestamp_nanos()).is_err())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let resolution = window
        .resolution
        .map(|_| crate::series::Resolution::QuarterHours);
    if resolution.is_some()
        && [from, to]
            .iter()
            .any(|t| t.unix_timestamp() % 900 != 0 || t.nanosecond() != 0)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    read_history(local.history_reader, move || {
        store
            .window_at_resolution(&point, from, to, resolution)
            .map(|window| Json(window).into_response())
            .map_err(|error| {
                tracing::warn!(%error, %point, "could not read measurement history");
                StatusCode::INTERNAL_SERVER_ERROR
            })
    })
    .await
}

/// Keep both query execution and JSON encoding within the single reader budget.
async fn read_history<T: Send + 'static>(
    reader: Arc<tokio::sync::Semaphore>,
    read: impl FnOnce() -> Result<T, StatusCode> + Send + 'static,
) -> Result<T, StatusCode> {
    let permit = reader
        .try_acquire_owned()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || {
        // The worker can outlive a disconnected HTTP client.
        let _permit = permit;
        read()
    })
    .await
    .map_err(|error| {
        tracing::error!(%error, "measurement history worker failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
}

/// What the household is asking for, and until when.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct OverrideBody {
    /// `boost`, `pause` or `away`.
    #[schemars(schema_with = "crate::openapi::user_override")]
    pub what: ems_core::setpoint::UserOverride,
    /// How long, in minutes. Absent takes the default; anything over a day is
    /// clamped, because longer than that is a statement about the house rather
    /// than about this afternoon.
    #[serde(default)]
    pub minutes: Option<i64>,
}

/// One override in force.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OverrideView {
    /// Which asset.
    pub asset: String,
    /// What was asked for.
    #[schemars(schema_with = "crate::openapi::user_override")]
    pub what: ems_core::setpoint::UserOverride,
    /// When it stops applying, RFC 3339.
    pub until: String,
}

/// Ask for one.
async fn set_override(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    axum::extract::Path(asset): axum::extract::Path<String>,
    Json(body): Json<OverrideBody>,
) -> Result<Json<OverrideView>, axum::http::StatusCode> {
    if local.status.lock().await.shadow {
        return Err(StatusCode::CONFLICT);
    }
    let asset =
        ems_core::prelude::AssetId::new(&asset).map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
    if body.what == ems_core::setpoint::UserOverride::Boost
        && local
            .home
            .site
            .as_ref()
            .is_some_and(|s| matches!(s.asset(&asset), Some(ems_core::prelude::Asset::Evse(_))))
    {
        return Err(StatusCode::CONFLICT);
    }
    let now = time::OffsetDateTime::now_utc();
    let held = local
        .overrides
        .set(
            asset.clone(),
            body.what,
            body.minutes.map(time::Duration::minutes),
            now,
        )
        .await;
    // Logged, because an override is a *decision somebody made* and the reason
    // chain a household is shown has to be able to say so.
    tracing::info!(
        %asset,
        what = ?held.what,
        until = %rfc3339(held.until),
        subject = %subject(who.as_deref()),
        "a household override"
    );
    Ok(Json(OverrideView {
        asset: asset.to_string(),
        what: held.what,
        until: rfc3339(held.until),
    }))
}

/// Withdraw one.
async fn clear_override(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    axum::extract::Path(asset): axum::extract::Path<String>,
) -> axum::http::StatusCode {
    let Ok(asset) = ems_core::prelude::AssetId::new(&asset) else {
        return axum::http::StatusCode::BAD_REQUEST;
    };
    if local.overrides.clear(&asset).await {
        tracing::info!(%asset, subject = %subject(who.as_deref()), "a household override was withdrawn");
        axum::http::StatusCode::NO_CONTENT
    } else {
        axum::http::StatusCode::NOT_FOUND
    }
}

/// Withdraw all of them — the "back to normal" button.
async fn clear_overrides(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
) -> Result<axum::http::StatusCode, (axum::http::StatusCode, String)> {
    let n = local.overrides.clear_all().await;
    if n == 0 {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "no override is in force".into(),
        ));
    }
    tracing::info!(withdrawn = n, subject = %subject(who.as_deref()), "every household override was withdrawn");
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// What is in force.
async fn list_overrides(State(local): State<Local>) -> Json<Vec<OverrideView>> {
    let now = time::OffsetDateTime::now_utc();
    Json(
        local
            .overrides
            .all(now)
            .await
            .into_iter()
            .map(|(asset, held)| OverrideView {
                asset: asset.to_string(),
                what: held.what,
                until: rfc3339(held.until),
            })
            .collect(),
    )
}

/// Who made a request, for the audit line (SR-007): `cem:<name>` or the site.
pub(super) fn subject(who: Option<&ems_service::auth::Authority>) -> String {
    who.map_or_else(|| "unknown".into(), |a| a.subject().to_string())
}

/// The household page's one document.
///
/// Built by [`crate::home::live`] from the same status, plan and prices every
/// other route reads, so the page cannot say something `/v1/status` does not.
async fn home(State(local): State<Local>) -> Result<Json<crate::home::Home>, StatusCode> {
    local
        .document()
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// The sender, or `404` on a box with no store: a subscription that would not
/// survive a restart is not one worth taking.
fn push_of(local: &Local) -> Result<&crate::runtime::push::Push, (StatusCode, String)> {
    local.push.as_ref().ok_or((
        StatusCode::NOT_FOUND,
        "this box keeps no store, so it cannot keep a subscription".into(),
    ))
}

/// A store failure, as an answer.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the shape `map_err` hands an error over in"
)]
fn push_store(error: crate::store::StoreError) -> (StatusCode, String) {
    tracing::error!(%error, "the push subscriptions could not be read or written");
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

// ── Events out ──────────────────────────────────────────────────────────────

fn hooks_of(local: &Local) -> Result<&crate::runtime::webhooks::Webhooks, (StatusCode, String)> {
    local.webhooks.as_ref().ok_or((
        StatusCode::NOT_FOUND,
        "this box sends no events to other systems".into(),
    ))
}

#[allow(clippy::needless_pass_by_value)] // the `map_err` shape: it takes what it is handed
fn hooks_refused(why: crate::runtime::webhooks::Refused) -> (StatusCode, String) {
    use crate::runtime::webhooks::Refused;
    let status = match &why {
        Refused::Destination(_) | Refused::UnknownType(_) => StatusCode::UNPROCESSABLE_ENTITY,
        Refused::Full => StatusCode::CONFLICT,
        Refused::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, why.to_string())
}

/// The systems this box tells about its events.
async fn list_subscriptions(
    State(local): State<Local>,
) -> Result<Json<Vec<crate::runtime::webhooks::View>>, (StatusCode, String)> {
    hooks_of(&local)?
        .list()
        .await
        .map(Json)
        .map_err(hooks_refused)
}

/// Tell one more system; its secret is in the answer and nowhere else.
async fn subscribe(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(asked): Json<crate::runtime::webhooks::Asked>,
) -> Result<(StatusCode, Json<crate::runtime::webhooks::Created>), (StatusCode, String)> {
    let created = hooks_of(&local)?
        .subscribe(asked, OffsetDateTime::now_utc())
        .await
        .map_err(hooks_refused)?;
    // The host and never the URL: a Home Assistant webhook id in the path is
    // that receiver's only credential, and a log is read by more people.
    let host = reqwest::Url::parse(&created.view.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    tracing::info!(
        name = %created.view.name,
        %host,
        subject = %subject(who.as_deref()),
        "a system will be told about this box's events"
    );
    Ok((StatusCode::CREATED, Json(created)))
}

/// Stop telling one.
async fn revoke_subscription(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if hooks_of(&local)?.revoke(&id).await.map_err(hooks_refused)? {
        tracing::info!(%id, subject = %subject(who.as_deref()), "a system will no longer be told");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no such subscription".into()))
    }
}

/// Send it a `de.ems.test.ping` now, and say what it answered.
async fn test_subscription(
    State(local): State<Local>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<crate::runtime::webhooks::TestDelivery>, (StatusCode, String)> {
    hooks_of(&local)?
        .test(&id, OffsetDateTime::now_utc())
        .await
        .map_err(hooks_refused)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "no such subscription".into()))
}

/// The last seven days of deliveries, newest first.
async fn deliveries(
    State(local): State<Local>,
) -> Result<Json<Vec<crate::runtime::webhooks::Delivery>>, (StatusCode, String)> {
    hooks_of(&local)?
        .deliveries(OffsetDateTime::now_utc())
        .await
        .map(Json)
        .map_err(hooks_refused)
}

/// The key a browser subscribes with, and the browsers subscribed.
async fn push_overview(
    State(local): State<Local>,
) -> Result<Json<crate::runtime::push::Overview>, (StatusCode, String)> {
    push_of(&local)?
        .overview()
        .await
        .map(Json)
        .map_err(push_store)
}

/// Subscribe this browser, or change what it asked for.
///
/// The body is the browser's own `PushSubscription.toJSON()` with the
/// household's choices beside it. Where a tap on a notification lands is the
/// address this request came to — the `Origin` a browser states — so it opens
/// the box the way this browser reaches it.
async fn push_subscribe(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    headers: axum::http::HeaderMap,
    Json(asked): Json<crate::runtime::push::Asked>,
) -> Result<Json<crate::runtime::push::View>, (StatusCode, String)> {
    use crate::runtime::push::Refused;
    let header = |name| {
        headers
            .get(name)
            .and_then(|v: &axum::http::HeaderValue| v.to_str().ok())
    };
    let origin = header(axum::http::header::ORIGIN)
        .filter(|o| o.starts_with("https://") || o.starts_with("http://"))
        .map(str::to_owned)
        .or_else(|| header(axum::http::header::HOST).map(|host| format!("https://{host}")))
        .unwrap_or_default();
    let view = push_of(&local)?
        .subscribe(asked, origin, OffsetDateTime::now_utc())
        .await
        .map_err(|refused| match refused {
            Refused::Store(error) => push_store(error),
            Refused::Full => (StatusCode::CONFLICT, refused.to_string()),
            Refused::Endpoint(_) | Refused::Keys(_) | Refused::Quiet => {
                (StatusCode::BAD_REQUEST, refused.to_string())
            }
        })?;
    tracing::info!(
        events = ?view.events,
        subject = %subject(who.as_deref()),
        "a browser subscribed to notifications"
    );
    Ok(Json(view))
}

/// Which browser a push request is about.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PushEndpoint {
    /// Its push endpoint, as `GET /v1/push` lists it.
    pub endpoint: String,
}

/// Unsubscribe a browser.
async fn push_forget(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(which): Json<PushEndpoint>,
) -> Result<StatusCode, (StatusCode, String)> {
    let existed = push_of(&local)?
        .forget(&which.endpoint)
        .await
        .map_err(push_store)?;
    if existed {
        tracing::info!(subject = %subject(who.as_deref()), "a browser unsubscribed from notifications");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no such subscription".into()))
    }
}

/// Send the test notification to one subscribed browser.
///
/// `204` once the push service has taken it; `410` where the push service says
/// the subscription has ended (it is then forgotten); `502` where the push
/// service refused or could not be reached.
async fn push_test(
    State(local): State<Local>,
    Json(which): Json<PushEndpoint>,
) -> Result<StatusCode, (StatusCode, String)> {
    use crate::runtime::push::Outcome;
    let outcome = push_of(&local)?
        .test(&which.endpoint, OffsetDateTime::now_utc())
        .await
        .map_err(push_store)?;
    match outcome {
        None => Err((StatusCode::NOT_FOUND, "no such subscription".into())),
        Some(Outcome::Accepted) => Ok(StatusCode::NO_CONTENT),
        Some(Outcome::Gone) => Err((
            StatusCode::GONE,
            "the browser's subscription has ended; subscribe again".into(),
        )),
        Some(Outcome::Refused(status)) => Err((
            StatusCode::BAD_GATEWAY,
            format!("the push service refused the notification ({status})"),
        )),
        Some(Outcome::Unreachable) => Err((
            StatusCode::BAD_GATEWAY,
            "the push service could not be reached from this box".into(),
        )),
    }
}

/// The closed days in the window asked for, read from the store.
///
/// `404` on a box with no store, `400` for a window that is not two dates in
/// order. The day in progress is never among them: a day is kept when it
/// closes.
async fn closed_days(
    local: &Local,
    window: &crate::month::Window,
) -> Result<crate::month::Days, StatusCode> {
    let Some(store) = local.days.as_ref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let kept = store.lock().await.closed_days().map_err(|error| {
        tracing::warn!(%error, "the closed days could not be read");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    window.of(kept).ok_or(StatusCode::BAD_REQUEST)
}

/// The closed days and their sum — what the month view draws.
async fn days(
    State(local): State<Local>,
    Query(window): Query<crate::month::Window>,
) -> Result<Json<crate::month::Days>, StatusCode> {
    closed_days(&local, &window).await.map(Json)
}

/// The same days as CSV, for the household to take with it.
async fn days_csv(
    State(local): State<Local>,
    Query(window): Query<crate::month::Window>,
) -> Result<axum::response::Response, StatusCode> {
    let found = closed_days(&local, &window).await?;
    Ok(crate::month::csv_response(&found.days))
}

/// The status a refused goal is answered with.
fn refused(why: crate::home::GoalRefused) -> (StatusCode, String) {
    use crate::home::GoalRefused as R;
    let status = match why {
        R::UnknownAsset => StatusCode::NOT_FOUND,
        R::NoGoalForThisKind | R::WrongShape => StatusCode::UNPROCESSABLE_ENTITY,
        R::OutOfRange => StatusCode::BAD_REQUEST,
    };
    (status, why.message().into())
}

/// A store that would not keep a goal.
fn not_kept(error: &crate::store::StoreError) -> (StatusCode, String) {
    tracing::error!(%error, "a goal could not be kept");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "the goal could not be kept".into(),
    )
}

#[derive(Default, serde::Deserialize)]
struct QuoteMode {
    #[serde(default)]
    charge_now: bool,
}

/// A price and saved goal the household has reviewed.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoostConfirmation {
    /// The exact stored goal, including the tariff cap.
    pub goal: crate::home::CarGoal,
    /// The difference from waiting, in whole cents.
    pub extra_eur: f64,
    /// The absolute deadline shown in the preview, Unix seconds.
    pub until: i64,
}

impl BoostConfirmation {
    fn matches(&self, quote: &crate::runtime::quote::Quote, now: OffsetDateTime) -> bool {
        quote.extra_eur == Some(self.extra_eur)
            && quote.boost_until == Some(self.until)
            && self.until > now.unix_timestamp()
    }
}

async fn boost_price(
    local: &Local,
    asset: &str,
    goal: crate::home::CarGoal,
) -> Result<crate::runtime::quote::Quote, (StatusCode, String)> {
    use crate::runtime::quote;
    let site = local
        .home
        .site
        .as_ref()
        .ok_or_else(|| refused(crate::home::GoalRefused::UnknownAsset))?;
    if crate::home::goal_kind(site, asset).map_err(refused)? != "car"
        || local.home.goal.get().car != Some(goal)
    {
        return Err((
            StatusCode::CONFLICT,
            "Review the saved car goal again".into(),
        ));
    }
    let unavailable = || {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "A priced, connected car is required".into(),
        )
    };
    let now = OffsetDateTime::now_utc();
    if local.status.lock().await.shadow
        || local
            .settings
            .as_ref()
            .is_some_and(|s| s.mode == crate::config::OperatingMode::Shadow)
    {
        return Err((StatusCode::CONFLICT, "This site is observing only".into()));
    }
    let registry = local.registry.as_ref().ok_or_else(unavailable)?;
    let id = ems_core::prelude::AssetId::new(asset).map_err(|_| unavailable())?;
    let seen = registry.lock().await.observe(None, now);
    let vehicle = seen
        .vehicles
        .get(&id)
        .filter(|_| !seen.silent.contains(&id))
        .ok_or_else(unavailable)?;
    let mut car = (*local.home.car.lock().await).ok_or_else(unavailable)?;
    let soc = vehicle
        .current_soc(now)
        .filter(|soc| *soc < goal.target_soc)
        .ok_or_else(unavailable)?;
    let capacity = vehicle
        .capacity_wh
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or_else(unavailable)?;
    car.capacity = ems_core::prelude::Energy::new(capacity);
    car.energy_now = car.capacity * soc;
    let mut held = quote::slot().ok_or((
        StatusCode::TOO_MANY_REQUESTS,
        "A quote is already running".into(),
    ))?;
    let last = local.home.last.clone();
    let mut price = tokio::spawn(async move {
        let result = quote::charge_now(&last, car, goal, now).await;
        *held = Some(std::time::Instant::now());
        result
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(unavailable)?;
    price.boost_until = Some(crate::home::next_wall_clock(goal.by_minute, now).unix_timestamp());
    Ok(price)
}

async fn start_boost(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Path(asset): Path<String>,
    Json(review): Json<BoostConfirmation>,
) -> Result<StatusCode, (StatusCode, String)> {
    let revision = local.home.goal.boost_revision();
    let quote = boost_price(&local, &asset, review.goal).await?;
    if !review.matches(&quote, OffsetDateTime::now_utc()) {
        return Err((
            StatusCode::CONFLICT,
            "The estimate changed. Review it again".into(),
        ));
    }
    let until = OffsetDateTime::from_unix_timestamp(review.until)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid deadline".into()))?;
    if !local
        .home
        .goal
        .start_boost(review.goal, until, revision)
        .await
    {
        return Err((StatusCode::CONFLICT, "The saved goal changed".into()));
    }
    tracing::info!(%asset, %until, subject = %subject(who.as_deref()), "household confirmed immediate charging");
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_boost(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Path(asset): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let site = local
        .home
        .site
        .as_ref()
        .ok_or_else(|| refused(crate::home::GoalRefused::UnknownAsset))?;
    if crate::home::goal_kind(site, &asset).map_err(refused)? != "car" {
        return Err(refused(crate::home::GoalRefused::NoGoalForThisKind));
    }
    local.home.goal.cancel_boost();
    tracing::info!(%asset, subject = %subject(who.as_deref()), "household cancelled immediate charging");
    Ok(StatusCode::NO_CONTENT)
}

/// What a proposed goal costs, using two solves of the same household.
async fn quote_goal(
    State(local): State<Local>,
    Query(mode): Query<QuoteMode>,
    Path(asset): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<crate::runtime::quote::Quote>, (StatusCode, String)> {
    use crate::home::NewGoal;
    let site = local
        .home
        .site
        .as_ref()
        .ok_or_else(|| refused(crate::home::GoalRefused::UnknownAsset))?;
    let goal = crate::home::parse_goal(site, &asset, body).map_err(refused)?;
    if mode.charge_now {
        return match goal {
            NewGoal::Car(g) => boost_price(&local, &asset, g).await.map(Json),
            NewGoal::Tank(_) => Err(refused(crate::home::GoalRefused::NoGoalForThisKind)),
        };
    }
    let Some(mut held) = crate::runtime::quote::slot() else {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "a quote is already being worked out; ask again in a moment".into(),
        ));
    };
    let now = OffsetDateTime::now_utc();
    let tank = site.assets.iter().find_map(|a| match a {
        ems_core::prelude::Asset::Dhw(tank) => Some(tank.clone()),
        _ => None,
    });
    let (last, car, tank_facts) = (
        local.home.last.clone(),
        local.home.car.clone(),
        local.home.tank.clone(),
    );
    // Its own task, holding the slot until both solves are done: a caller that
    // walks away drops this handler, and must not free the slot for two more
    // solves while the first two still run.
    let quote = tokio::spawn(async move {
        let quote = match goal {
            NewGoal::Car(g) => crate::runtime::quote::car(&last, &car, g, now).await,
            NewGoal::Tank(g) => match &tank {
                Some(tank) => crate::runtime::quote::tank(&last, &tank_facts, tank, g, now).await,
                None => None,
            },
        };
        *held = Some(std::time::Instant::now());
        drop(held);
        quote
    })
    .await
    .ok()
    .flatten();
    quote.map(Json).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "the box has not planned yet, or cannot see this device".into(),
    ))
}

async fn set_goal(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Path(asset): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use crate::home::NewGoal;
    let site = local
        .home
        .site
        .as_ref()
        .ok_or_else(|| refused(crate::home::GoalRefused::UnknownAsset))?;
    let goal = crate::home::parse_goal(site, &asset, body).map_err(refused)?;
    local
        .home
        .goal
        .change(|set| match goal {
            NewGoal::Car(g) => set.car = Some(g),
            NewGoal::Tank(g) => set.tank = Some(g),
        })
        .await
        .map_err(|e| not_kept(&e))?;
    tracing::info!(%asset, ?goal, subject = %subject(who.as_deref()), "a goal");
    let now = OffsetDateTime::now_utc();
    let answer = match goal {
        NewGoal::Car(g) => {
            let prices = local.home.prices.read().await;
            local.home.car.lock().await.map(|facts| {
                serde_json::json!(crate::home::judge_car_price(
                    &crate::home::session_for(&facts, g, now),
                    ems_core::prelude::Slot::containing(now),
                    g.price_cap_ct,
                    prices.as_ref(),
                ))
            })
        }
        NewGoal::Tank(g) => {
            let tank = site.assets.iter().find_map(|a| match a {
                ems_core::prelude::Asset::Dhw(tank) => Some(tank),
                _ => None,
            });
            let facts = *local.home.tank.lock().await;
            tank.zip(facts).map(|(tank, facts)| {
                serde_json::json!(crate::home::judge_tank(tank, &facts, g, now))
            })
        }
    };
    Ok(Json(answer.unwrap_or(serde_json::Value::Null)))
}

/// Withdraw a goal; for the car, the configured departure applies again.
async fn clear_goal(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Path(asset): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let site = local
        .home
        .site
        .as_ref()
        .ok_or_else(|| refused(crate::home::GoalRefused::UnknownAsset))?;
    let kind = crate::home::goal_kind(site, &asset).map_err(refused)?;
    let changed = local
        .home
        .goal
        .change(|set| {
            if kind == "car" {
                set.car = None;
            } else {
                set.tank = None;
            }
        })
        .await
        .map_err(|e| not_kept(&e))?;
    Ok(if changed {
        tracing::info!(%asset, subject = %subject(who.as_deref()), "a household goal was withdrawn");
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

/// RFC 3339, or an empty string for an instant that cannot be formatted.
fn rfc3339(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// What the box is doing.
async fn status(State(local): State<Local>) -> Json<StatusBody> {
    let held = local.status.lock().await;
    Json(StatusBody {
        site: local.site.clone(),
        ski: local.ski.clone(),
        at: held.at.map(rfc3339),
        commanded_w: held
            .commanded
            .iter()
            .map(|(id, p)| (id.to_string(), p.get()))
            .collect(),
        silent: held.silent.iter().map(ToString::to_string).collect(),
        assumed_available: held
            .assumed_available
            .iter()
            .map(ToString::to_string)
            .collect(),
        assumed_nominal: held
            .assumed_nominal
            .iter()
            .map(ToString::to_string)
            .collect(),
        disobedient: held
            .disobedient
            .iter()
            .map(|(id, why)| (id.to_string(), why.clone()))
            .collect(),
        undriven: held.undriven.iter().map(ToString::to_string).collect(),
        roof_health: held.roof_health,
        steuve_ceiling_kw: held.steuve_ceiling.map(ems_core::prelude::Power::kw),
        steuve_budget_kw: held.steuve_budget.map(ems_core::prelude::Power::kw),
        netzwirksam_kw: held.netzwirksam.map(ems_core::prelude::Power::kw),
        balance_residual_w: held.balance_residual.map(ems_core::prelude::Power::get),
        // `i64::MAX` is the sentinel for "never planned", and it must not reach
        // a screen as a number: nine quintillion minutes is not a duration
        // anybody can act on, and `null` is the honest JSON for "there has not
        // been one".
        minutes_without_a_plan: (held.minutes_without_a_plan != i64::MAX)
            .then_some(held.minutes_without_a_plan),
        plan_expected_eur: held.plan_expected_eur,
        overruns: held.overruns,
        cem: held.cem.clone(),
        overriding: held
            .overriding
            .iter()
            .map(|(id, rule)| (id.to_string(), rule.clone()))
            .collect(),
        exposure: held.exposure.as_ref().map(|e| ExposureBody {
            days_of_record: e.days_of_record,
            hours_reduced: e.hours_reduced,
            busiest_quarter_hour: e.busiest_quarter_hour.map(quarter_hour),
            busiest_frequency: e.busiest_frequency,
            typical_ceiling_kw: e.typical_ceiling_kw,
        }),
    })
}

/// A quarter-hour index of the local day as the clock face it names.
///
/// `71` is not a time anybody reads; `17:45` is.
fn quarter_hour(index: u32) -> String {
    format!("{:02}:{:02}", index / 4, (index % 4) * 15)
}

#[cfg(test)]
mod exposure_tests {
    use super::*;
    use crate::runtime::control::Exposure;

    /// The figures survive the transcription into JSON.
    ///
    /// A DTO built field by field out of a domain struct is the shape that
    /// loses one silently: the field is added to `Status`, the control loop
    /// fills it, and the handler that was written before it exists carries on
    /// serialising everything else. `hours_reduced` reading zero on a household
    /// that has been reduced for eleven hours looks like a household nobody
    /// touched.
    #[tokio::test]
    async fn what_the_box_learned_about_its_own_exposure_reaches_the_page() {
        let held = Status {
            exposure: Some(Exposure {
                days_of_record: 90,
                hours_reduced: 11.5,
                busiest_quarter_hour: Some(71),
                busiest_frequency: 0.8,
                typical_ceiling_kw: Some(4.2),
            }),
            ..Status::default()
        };
        let local = Local::new(
            Arc::new(Mutex::new(held)),
            "haus".into(),
            None,
            crate::runtime::overrides::Overrides::default(),
            None,
            crate::runtime::access::LocalAccess::for_testing("haus", "t"),
            None,
        );

        let Json(body) = status(State(local)).await;
        let seen = body.exposure.expect("an exposure that was set");

        assert_eq!(seen.days_of_record, 90);
        assert!((seen.hours_reduced - 11.5).abs() < f64::EPSILON);
        assert!((seen.busiest_frequency - 0.8).abs() < f64::EPSILON);
        assert_eq!(seen.typical_ceiling_kw, Some(4.2));
        // Quarter hour 71 of the local day is a quarter to six in the evening,
        // and 71 is not a time anybody can act on.
        assert_eq!(seen.busiest_quarter_hour.as_deref(), Some("17:45"));
    }

    /// A box that has never been reduced says so with `null` rather than zeroes.
    ///
    /// Nought hours out of nought days is not the same claim as nought hours out
    /// of ninety, and only one of them is evidence.
    #[tokio::test]
    async fn a_box_with_no_record_reports_nothing_rather_than_a_clean_sheet() {
        let local = Local::new(
            Arc::new(Mutex::new(Status::default())),
            "haus".into(),
            None,
            crate::runtime::overrides::Overrides::default(),
            None,
            crate::runtime::access::LocalAccess::for_testing("haus", "t"),
            None,
        );

        let Json(body) = status(State(local)).await;

        assert!(body.exposure.is_none());
    }
}

/// Who this box is, who it will talk to, and who is asking.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PairingBody {
    /// This box's own SKI — what the metering point operator has to be given.
    pub ski: Option<String>,
    /// The SKIs this box will exchange data with.
    pub trusted: Vec<String>,
    /// The peers whose handshake is **waiting on a decision right now**.
    ///
    /// The third side of the commissioning exchange and the one that used to be
    /// missing. An unapproved peer completes TLS — so its SKI is *proved* rather
    /// than claimed — and SHIP holds it pending precisely so a person can compare
    /// it with the label on the device in front of them. Without it the SKI had
    /// to be read off the Steuerbox instead, which is the step field reports name
    /// as the most common § 14a commissioning failure.
    pub waiting: Vec<crate::runtime::ship::Waiting>,
}

/// A SKI an installer has read off a Steuerbox.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct Approval {
    /// Forty hexadecimal characters, as printed on the peer.
    pub ski: String,
}

/// Both halves of the § 14a commissioning exchange, on one page.
///
/// The two directions fail differently and an installer needs both in front of
/// them: this box's SKI is what the metering point operator has to be given, and
/// the trusted list is what this box will accept.
async fn list_trusted(State(local): State<Local>) -> Json<PairingBody> {
    Json(PairingBody {
        ski: local.ski.clone(),
        trusted: local.trust.as_ref().map(Trust::peers).unwrap_or_default(),
        waiting: local.trust.as_ref().map(Trust::waiting).unwrap_or_default(),
    })
}

/// Approve a Steuerbox, on a box that is already running.
///
/// The session it completes may already be waiting: an unapproved peer completes
/// TLS — which proves its SKI rather than taking its word — and is held in the
/// SHIP pending state, and an approval added meanwhile lets it through. So an
/// installer approves the SKI printed on the Steuerbox and the reduction path is
/// live, with no restart and no edited file.
async fn approve_peer(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(approval): Json<Approval>,
) -> Result<Json<PairingBody>, (axum::http::StatusCode, String)> {
    let Some(trust) = local.trust.as_ref() else {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "this box has no EEBUS identity, so there is nothing to pair with".into(),
        ));
    };
    trust
        .approve(&approval.ski, OffsetDateTime::now_utc())
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
    // Who the box obeys is a decision somebody made, and the log says who.
    // `?ski`, not `%`: it is the caller's text, and `Debug` escapes it.
    tracing::info!(ski = ?approval.ski, subject = %subject(who.as_deref()), "a control box was approved");
    Ok(list_trusted(State(local)).await)
}

/// Turn down a peer that is waiting.
///
/// Distinct from [`forget_peer`]: this one is about a handshake happening *now*,
/// and answering it means the peer aborts with `hello: aborted` and learns it was
/// refused instead of timing out. It says nothing about the future — the peer may
/// ask again — and nothing at all about a peer that is not currently waiting.
async fn refuse_peer(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    axum::extract::Path(ski): axum::extract::Path<String>,
) -> Result<Json<PairingBody>, (axum::http::StatusCode, String)> {
    let Some(trust) = local.trust.as_ref() else {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "this box has no EEBUS identity, so there is nothing to pair with".into(),
        ));
    };
    trust
        .refuse(&ski)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
    tracing::info!(?ski, subject = %subject(who.as_deref()), "a waiting control box was refused");
    Ok(list_trusted(State(local)).await)
}

/// Withdraw approval from a peer.
///
/// Its session is not torn down: the § 14a session is how a reduction arrives,
/// and dropping it the instant somebody revokes a SKI would take the household
/// out of contact with its network operator on a keystroke. It cannot
/// reconnect, which is what revocation means.
async fn forget_peer(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    axum::extract::Path(ski): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, (axum::http::StatusCode, String)> {
    let Some(trust) = local.trust.as_ref() else {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "this box has no EEBUS identity, so there is nothing to pair with".into(),
        ));
    };
    let wanted = ski.parse::<eebus::ship::Ski>().map_err(|_| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("{ski} is not a SKI: forty hexadecimal digits"),
        )
    })?;
    let known = trust
        .peers()
        .iter()
        .any(|t| t.parse::<eebus::ship::Ski>().is_ok_and(|t| t == wanted));
    if !known {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "this box does not trust that control box, so there is nothing to forget".into(),
        ));
    }
    trust
        .forget(&ski, OffsetDateTime::now_utc())
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tracing::info!(?ski, subject = %subject(who.as_deref()), "a control box was forgotten");
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ── Commissioning: the codes, the sheet, discovery and the trust check ──────

/// What a QR code may be asked to carry.
#[derive(Debug, serde::Deserialize)]
struct QrQuery {
    /// For `signin`: the origin the page is open at, `https://host[:port]`.
    /// The request's own `Host` where absent.
    origin: Option<String>,
    /// For `commissioning`: the caller's own identifier, echoed in the record.
    reference: Option<String>,
}

/// A QR code, as SVG: `ski` (the SHIP pairing payload — SKI, SHIP ID and
/// certificate fingerprint), `fingerprint` (the fingerprint alone) or `signin`
/// (a link that signs the page in).
///
/// **`signin` answers only a request that presented the household's token
/// itself**, not one riding on the session cookie. The cookie is `HttpOnly` so
/// that nothing running in the page can read the credential; a route that
/// handed the token to any signed-in request would undo exactly that. Whoever
/// asks for this code already holds what it encodes.
async fn qr(
    State(local): State<Local>,
    Path(what): Path<String>,
    Query(query): Query<QrQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, StatusCode> {
    use crate::runtime::handover;
    let text = match what.as_str() {
        "ski" => {
            let ski = local.ski.as_deref().ok_or(StatusCode::NOT_FOUND)?;
            let ship_id = local
                .commissioning
                .ship_id
                .as_deref()
                .ok_or(StatusCode::NOT_FOUND)?;
            handover::ship_payload(ski, ship_id, local.home.fingerprint.as_deref())
                .ok_or(StatusCode::NOT_FOUND)?
        }
        "fingerprint" => local
            .home
            .fingerprint
            .clone()
            .ok_or(StatusCode::NOT_FOUND)?,
        "signin" => {
            if !headers.contains_key(axum::http::header::AUTHORIZATION) {
                return Err(StatusCode::FORBIDDEN);
            }
            let origin = if let Some(origin) = query.origin {
                origin
            } else {
                let host = headers
                    .get(axum::http::header::HOST)
                    .and_then(|h| h.to_str().ok())
                    .filter(|h| handover::is_host(h))
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let scheme = if local.home.https_fingerprint.is_some() {
                    "https"
                } else {
                    "http"
                };
                format!("{scheme}://{host}")
            };
            if !handover::is_origin(&origin) {
                return Err(StatusCode::BAD_REQUEST);
            }
            handover::sign_in_link(&origin, &local.access.household_token())
        }
        "commissioning" => {
            let jws = record_jws(&local, query.reference, false)
                .await
                .map_err(|(status, _)| status)?;
            return Ok(match handover::qr_svg_dense(&jws) {
                Ok(svg) => svg_response(svg),
                // Not an error of the box: a large installation's record is
                // fetched, and the page offers that instead.
                Err(handover::TooDense { octets }) => {
                    use axum::response::IntoResponse;
                    (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!(
                            "this record is {octets} octets, more than the {} a code is \
                             scanned at reliably; fetch /v1/commissioning instead",
                            handover::MAX_CODE_OCTETS
                        ),
                    )
                        .into_response()
                }
            });
        }
        _ => return Err(StatusCode::NOT_FOUND),
    };
    let svg = handover::qr_svg(&text).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(svg_response(svg))
}

/// A QR code's answer: never cached, never sniffed.
fn svg_response(svg: String) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
            // One of these is a credential, and none of them is worth a cache
            // entry on a shared tablet.
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        svg,
    )
        .into_response()
}

/// Which commissioning record to sign.
#[derive(Debug, serde::Deserialize)]
struct RecordQuery {
    /// The caller's own identifier for the work, echoed byte for byte.
    reference: Option<String>,
}

/// The signed commissioning record: what a field-service tool takes away as
/// proof of what the box said about itself. See [`crate::runtime::handover`].
async fn commissioning(
    State(local): State<Local>,
    Query(query): Query<RecordQuery>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::response::IntoResponse;
    let jws = record_jws(&local, query.reference, true).await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/jose"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        jws,
    )
        .into_response())
}

/// The commissioning record as a JWS, signed with the box's identity key.
async fn record_jws(
    local: &Local,
    reference: Option<String>,
    carry_certificate: bool,
) -> Result<String, (StatusCode, String)> {
    use crate::runtime::handover;
    let none = || {
        (
            StatusCode::NOT_FOUND,
            "this box has no EEBUS identity to sign with".to_owned(),
        )
    };
    if reference
        .as_deref()
        .is_some_and(|r| !handover::is_reference(r))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "a reference is 1 to 64 printable ASCII characters without a space".into(),
        ));
    }
    let trust = local.trust.as_ref().ok_or_else(none)?;
    let site = local.home.site.as_ref().ok_or_else(none)?;
    let certificate = trust.certificate_der().ok_or_else(none)?;
    let sheet = handover::sheet(
        site,
        OffsetDateTime::now_utc(),
        local.ski.as_deref(),
        local.home.fingerprint.as_deref(),
        local.home.https_fingerprint.as_deref(),
        &local.commissioning,
    );
    let peers = trust.peers();
    let control_box = match peers.as_slice() {
        [one] => Some(one.as_str()),
        _ => None,
    };
    let record = handover::record(&sheet, &trust_of(local).await, control_box, reference)
        .ok_or_else(none)?;
    // A code carries the record without its certificate and without the box's
    // own device names: with them it is too dense to scan off a phone, and
    // whoever verifies it already holds the certificate or the SKI.
    let record = if carry_certificate {
        record
    } else {
        record.for_a_code()
    };
    handover::jws(&record, &certificate, carry_certificate, |input| {
        trust.sign_record(input)
    })
    .ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "the record could not be signed".into(),
    ))
}

/// The handover sheet's one document.
async fn handover(
    State(local): State<Local>,
) -> Result<Json<crate::runtime::handover::Handover>, StatusCode> {
    let site = local.home.site.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(crate::runtime::handover::sheet(
        site,
        OffsetDateTime::now_utc(),
        local.ski.as_deref(),
        local.home.fingerprint.as_deref(),
        local.home.https_fingerprint.as_deref(),
        &local.commissioning,
    )))
}

/// Look for devices: the configured ones from what the box hears, the named
/// hosts by a SunSpec probe, SHIP nodes by their announcements.
///
/// Narrow by construction — see [`crate::runtime::handover`].
async fn discover(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(ask): Json<crate::runtime::handover::Ask>,
) -> Result<Json<crate::runtime::handover::Discovered>, (StatusCode, String)> {
    use crate::runtime::handover;
    let hosts = handover::hosts(&ask).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Logged with who asked: this is the box opening sockets on request.
    tracing::info!(hosts = ?hosts, subject = %subject(who.as_deref()), "a discovery was asked for");
    let known = handover::Known {
        own_ski: local.ski.clone(),
        trusted: local.trust.as_ref().map(Trust::peers).unwrap_or_default(),
        waiting: local
            .trust
            .as_ref()
            .map(Trust::waiting)
            .unwrap_or_default()
            .into_iter()
            .map(|w| w.ski)
            .collect(),
        silent: local.status.lock().await.silent.iter().cloned().collect(),
    };
    let unit = ask.unit.unwrap_or(1);
    let port = ask.port.unwrap_or(handover::MODBUS_PORTS[0]);
    let mut found = handover::discover(&local.commissioning, &known, &hosts, unit, port).await;
    if let Some(settings) = local.settings.as_ref() {
        for device in &mut found.modbus {
            if let Some(identity) = device
                .identity
                .as_ref()
                .filter(|id| id.supports_pv_driver())
                && let Ok(ip) = device.host.parse()
            {
                let mut review = handover::InverterReview::new(
                    settings,
                    std::net::SocketAddr::new(ip, port),
                    unit,
                );
                if let Some((installer, _)) = &local.installer {
                    review.proposal = installer.offer(&mut review, identity);
                }
                device.review = Some(review);
            }
        }
    }
    Ok(Json(found))
}

/// Only the server-issued id is accepted. The TOML is held by the gateway.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyInstallation {
    /// The id shown with the reviewed proposal.
    pub id: String,
}

async fn handover_evidence(local: &Local) -> Result<super::authority::Readiness, StatusCode> {
    let registry = local.registry.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let site = local.home.site.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let meter = site.assets.iter().find(|a| matches!(a, ems_core::prelude::Asset::Meter(m) if m.role == ems_core::asset::MeterRole::GridConnection)).map(ems_core::prelude::Asset::id);
    let observed = registry
        .lock()
        .await
        .observe(meter, OffsetDateTime::now_utc());
    Ok(super::authority::readiness(site, &observed))
}

async fn control_transfer(
    State(local): State<Local>,
) -> Result<Json<super::authority::Review>, StatusCode> {
    let store = local.days.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let transfer = store
        .lock()
        .await
        .transfer()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(super::authority::Review {
        transfer,
        readiness: handover_evidence(&local).await?,
    }))
}

async fn review_transfer(
    State(local): State<Local>,
    Json(request): Json<super::authority::Approval>,
) -> Result<Json<super::authority::Review>, StatusCode> {
    if local
        .settings
        .as_ref()
        .is_none_or(|s| s.mode != crate::config::OperatingMode::Shadow)
    {
        return Err(StatusCode::CONFLICT);
    }
    let readiness = handover_evidence(&local).await?;
    if request.approve && readiness != super::authority::Readiness::Ready {
        return Err(StatusCode::CONFLICT);
    }
    let store = local
        .days
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)?
        .lock()
        .await;
    let mut transfer = store
        .transfer()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if !transfer.review(&request, time::OffsetDateTime::now_utc().unix_timestamp()) {
        return Err(StatusCode::CONFLICT);
    }
    store
        .put_transfer(Some(&transfer))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(super::authority::Review {
        transfer,
        readiness,
    }))
}

async fn installation_outcome(
    State(local): State<Local>,
) -> Result<Json<Option<super::installation::Receipt>>, StatusCode> {
    let Some((installer, _)) = &local.installer else {
        return Err(StatusCode::NOT_FOUND);
    };
    installer
        .receipt()
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn apply_installation(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(request): Json<ApplyInstallation>,
) -> Result<Json<super::installation::Receipt>, (StatusCode, String)> {
    let (installer, stop) = local.installer.ok_or((
        StatusCode::NOT_FOUND,
        "Installation changes are unavailable".to_owned(),
    ))?;
    commit_installation(installer, stop, who.as_deref(), request).await
}

pub(super) async fn commit_installation(
    installer: std::sync::Arc<super::installation::Installer>,
    stop: ems_service::shutdown::ShutdownTrigger,
    actor: Option<&ems_service::auth::Authority>,
    request: ApplyInstallation,
) -> Result<Json<super::installation::Receipt>, (StatusCode, String)> {
    use super::installation::ApplyError;
    let (receipt, restart) = tokio::task::spawn_blocking(move || installer.apply(&request.id))
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Installation task stopped".to_owned(),
            )
        })?
        .map_err(|error| {
            let status = match error {
                ApplyError::Stale => StatusCode::CONFLICT,
                ApplyError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, error.to_string())
        })?;
    if restart {
        tracing::info!(subject = %subject(actor), change = %receipt.id, role = ?receipt.role, "household confirmed installation");
        stop.trigger();
    }
    Ok(Json(receipt))
}

/// The household's new token, once.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct NewToken {
    /// What the household now signs in with. The old one is dead.
    pub token: String,
}

/// Replace the household's token: every session, every sign-in code and every
/// printed handover sheet made before stops working.
///
/// Like the sign-in code, it answers only a request that **presented the
/// token**, not one riding on the cookie: the answer is a credential, and the
/// cookie is `HttpOnly` precisely so the page's scripts never hold one.
async fn replace_token(
    State(local): State<Local>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use crate::runtime::access::ReplaceError;
    use axum::response::IntoResponse;
    if !headers.contains_key(axum::http::header::AUTHORIZATION) {
        return Err((
            StatusCode::FORBIDDEN,
            "present the current token itself to replace it".into(),
        ));
    }
    let token = local.access.replace_token().await.map_err(|error| {
        let status = match error {
            ReplaceError::Configured => StatusCode::CONFLICT,
            ReplaceError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, error.to_string())
    })?;
    tracing::warn!(subject = %subject(who.as_deref()), "the household's token was replaced");
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(NewToken { token }),
    )
        .into_response())
}

/// Whether the box is who it was, and whether its control box is still there.
async fn trust_check(State(local): State<Local>) -> Json<crate::doctor::Diagnostics> {
    use crate::doctor::HistoryState;
    let history = if let Some(store) = local.series.clone() {
        match read_history(local.history_reader.clone(), move || {
            store.repair_pending().map_err(|error| {
                tracing::warn!(%error, "could not inspect quarter-hour repair status");
                StatusCode::INTERNAL_SERVER_ERROR
            })
        })
        .await
        {
            Ok(true) => HistoryState::RepairPending,
            Ok(false) => HistoryState::Current,
            Err(StatusCode::SERVICE_UNAVAILABLE) => HistoryState::Busy,
            Err(_) => HistoryState::Unavailable,
        }
    } else {
        HistoryState::Disabled
    };
    let serial_meters = if let Some(registry) = &local.registry {
        let registry = registry.lock().await;
        let now = time::OffsetDateTime::now_utc();
        local
            .commissioning
            .serial
            .iter()
            .map(|(asset, bus, unit)| registry.serial_reading(asset, bus, *unit, now))
            .collect()
    } else {
        Vec::new()
    };
    Json(crate::doctor::Diagnostics {
        serial_meters,
        heat_meter: local
            .heat_meter
            .as_ref()
            .map(super::heat_meter::Observer::reading),
        trust: trust_of(&local).await,
        history,
    })
}

fn control_box_state(
    asset: Option<&ems_core::prelude::AssetId>,
    trust: Option<&super::ship::Trust>,
    status: &Status,
) -> super::handover::ControlBox {
    super::handover::control_box(super::handover::ControlBoxFacts {
        configured: asset.is_some() && trust.is_some(),
        trusted: trust.map_or(0, |t| t.peers().len()),
        waiting: trust.map_or(0, |t| t.waiting().len()),
        silent: status.at.is_none() || asset.is_some_and(|id| status.silent.contains(id)),
        in_failsafe: status.in_failsafe,
    })
}

/// The trust check, for the route and for the commissioning record.
async fn trust_of(local: &Local) -> crate::runtime::handover::TrustCheck {
    use crate::runtime::handover;
    let identity = match &local.trust {
        None => "none",
        Some(trust) if trust.identity_is_kept().await => "kept",
        Some(_) => "ephemeral",
    };
    let (tick_at, devices, control_box) = {
        let status = local.status.lock().await;
        (
            status.at,
            crate::doctor::devices(&local.commissioning, &status),
            control_box_state(
                local.commissioning.control_box.as_ref(),
                local.trust.as_ref(),
                &status,
            ),
        )
    };
    let last_heartbeat = match (&local.registry, &local.commissioning.control_box) {
        (Some(registry), Some(asset)) => registry.lock().await.last_lpc_heartbeat(asset),
        _ => None,
    };
    handover::TrustCheck {
        support_contact: local
            .settings
            .as_ref()
            .and_then(|s| s.support_contact.clone()),
        heartbeat_age_s: last_heartbeat.and_then(|at| {
            u64::try_from((time::OffsetDateTime::now_utc() - at).whole_seconds()).ok()
        }),
        tick_at: tick_at.map(rfc3339),
        devices,
        identity,
        ski: local.ski.clone(),
        fingerprint: local.home.fingerprint.clone(),
        https_fingerprint: local.home.https_fingerprint.clone(),
        control_box,
    }
}

#[cfg(test)]
mod series_tests {
    use super::*;

    #[test]
    fn boost_confirmation_requires_the_current_price_and_deadline() {
        let now = time::macros::datetime!(2026-01-16 10:00 UTC);
        let until = (now + time::Duration::HOUR).unix_timestamp();
        let mut review = BoostConfirmation {
            goal: crate::home::CarGoal {
                target_soc: 0.8,
                by_minute: 720,
                price_cap_ct: None,
            },
            extra_eur: 0.45,
            until,
        };
        let quote = crate::runtime::quote::Quote {
            verdict: crate::home::Verdict::Feasible,
            reachable: None,
            extra_eur: Some(0.45),
            eligible_quarters: None,
            boost_until: Some(until),
        };
        assert!(review.matches(&quote, now));
        assert!(!review.matches(&quote, now + time::Duration::HOUR));
        review.extra_eur = 0.44;
        assert!(!review.matches(&quote, now));
        review.extra_eur = 0.45;
        review.until += 1;
        assert!(!review.matches(&quote, now));
    }

    #[tokio::test]
    async fn a_box_that_keeps_no_series_says_so_rather_than_answering_nothing() {
        // `404`, not an empty array. *Nothing was kept* and *nothing happened*
        // are different answers, and an empty list gives the second to a
        // household asking the first.
        let local = Local::new(
            Arc::new(Mutex::new(Status::default())),
            "haus".into(),
            None,
            crate::runtime::overrides::Overrides::default(),
            None,
            crate::runtime::access::LocalAccess::for_testing("haus", "t"),
            None,
        );
        assert_eq!(
            trust_check(State(local.clone())).await.history,
            crate::doctor::HistoryState::Disabled
        );
        let outcome = series(
            State(local),
            Path("grid".to_owned()),
            Query(Asked {
                resolution: None,
                from: None,
                to: None,
            }),
        )
        .await;
        assert_eq!(outcome.err(), Some(StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn the_household_reads_back_the_history_its_own_box_took() {
        // The Data Act half of the box: the user's own data, from the process
        // that measured it, over the local API rather than a fleet service.
        let dir = std::env::temp_dir().join(format!("hems-api-series-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::series::Series::open(&dir, 7).expect("a series");
        let at = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
        store
            .record(
                at,
                &[(crate::series::GRID, ems_core::prelude::Power::from_kw(2.5))],
            )
            .expect("a tick");

        let local = Local::new(
            Arc::new(Mutex::new(Status::default())),
            "haus".into(),
            None,
            crate::runtime::overrides::Overrides::default(),
            None,
            crate::runtime::access::LocalAccess::for_testing("haus", "t"),
            Some(Arc::new(store)),
        );
        {
            let _permit = local.history_reader.acquire().await.unwrap();
            assert_eq!(
                trust_check(State(local.clone())).await.history,
                crate::doctor::HistoryState::Busy
            );
        }
        assert_eq!(
            trust_check(State(local.clone())).await.history,
            crate::doctor::HistoryState::Current
        );
        // No window: the default hour back is what a screen asks for, and it has
        // to be wide enough to contain a reading five minutes old.
        let start = ems_core::prelude::Slot::containing(at).start();
        let quarter = time::Duration::minutes(15);
        for (from, to) in [
            (start, start),
            (start, start + quarter + time::Duration::NANOSECOND),
            (start, start + time::Duration::days(1097)),
            (
                time::macros::datetime!(1400-01-01 00:00 UTC),
                time::macros::datetime!(1400-01-02 00:00 UTC),
            ),
        ] {
            let asked = Asked {
                from: Some(from),
                to: Some(to),
                resolution: Some(HistoryResolution::QuarterHours),
            };
            assert_eq!(
                series(State(local.clone()), Path("grid".into()), Query(asked))
                    .await
                    .err(),
                Some(StatusCode::BAD_REQUEST)
            );
        }
        let asked = Asked {
            from: Some(start),
            to: Some(start + quarter),
            resolution: Some(HistoryResolution::QuarterHours),
        };
        let response = series(State(local.clone()), Path("grid".into()), Query(asked))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let summary: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(summary["resolution"], "quarter_hours");
        assert_eq!(summary["readings"][0]["samples"], 1);

        let latest_closed = Asked {
            from: None,
            to: None,
            resolution: Some(HistoryResolution::QuarterHours),
        };
        assert!(
            series(
                State(local.clone()),
                Path("grid".into()),
                Query(latest_closed)
            )
            .await
            .is_ok()
        );
        let response = series(
            State(local),
            Path(crate::series::GRID.to_owned()),
            Query(Asked {
                resolution: None,
                from: None,
                to: None,
            }),
        )
        .await
        .expect("a box with a series answers");
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let window: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(window["resolution"], "seconds");
        assert_eq!(window["readings"].as_array().unwrap().len(), 1);
        assert_eq!(window["readings"][0]["watts"], 2500.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    async fn cancelling_a_history_request_keeps_the_worker_bounded_until_it_finishes() {
        let reader = Arc::new(tokio::sync::Semaphore::new(1));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let pending = tokio::spawn(read_history(reader.clone(), move || {
            started.send(()).unwrap();
            blocked
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(())
        }));
        ready.await.unwrap();
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert_eq!(
            read_history(reader.clone(), || Ok(())).await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        release.send(()).unwrap();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), reader.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert!(read_history(reader, || Ok(())).await.is_ok());
    }

    /// A box that knows the reference household, for the goal routes.
    fn with_the_reference_house() -> (Local, String, String) {
        let site = crate::site::Household::build(&crate::HouseholdConfig::default())
            .expect("the reference household")
            .site;
        let id = |kind: &str| {
            crate::home::devices_of(&site)
                .into_iter()
                .find(|(_, k)| *k == kind)
                .map(|(id, _)| id.to_string())
                .expect("the reference household has one")
        };
        let (car, tank) = (id("car"), id("hot_water"));
        let local = Local::new(
            Arc::new(Mutex::new(Status::default())),
            "haus".into(),
            None,
            crate::runtime::overrides::Overrides::default(),
            None,
            crate::runtime::access::LocalAccess::for_testing("haus", "t"),
            None,
        )
        .with_home(HomeSources {
            site: Some(Arc::new(site)),
            ..HomeSources::default()
        });
        (local, car, tank)
    }

    #[tokio::test]
    async fn a_failed_replan_identifies_the_retained_plan_until_recovery() {
        use ems_core::prelude::{Horizon, Plan};

        let (local, _, _) = with_the_reference_house();
        let at = OffsetDateTime::now_utc();
        local.home.health.bad("planner", "no prices");
        assert!(local.document().await.unwrap().previous_plan_at.is_none());
        *local.home.plan.write().await = Some(Plan::empty(Horizon::new(at, 1), at));
        assert_eq!(
            local.document().await.unwrap().previous_plan_at,
            Some(rfc3339(at))
        );
        local.home.health.good("planner", at);
        assert!(local.document().await.unwrap().previous_plan_at.is_none());
    }

    #[tokio::test]
    async fn a_goal_is_refused_when_it_is_out_of_range_or_for_the_wrong_device() {
        let (local, car, tank) = with_the_reference_house();
        let put = |asset: &str, body: serde_json::Value| {
            set_goal(
                State(local.clone()),
                None,
                Path(asset.to_owned()),
                Json(body),
            )
        };
        let status = |outcome: Result<_, (StatusCode, String)>| outcome.err().map(|(s, _)| s);

        assert_eq!(
            status(
                put(
                    &car,
                    serde_json::json!({ "target_soc": 1.5, "by_minute": 420 })
                )
                .await
            ),
            Some(StatusCode::BAD_REQUEST)
        );
        assert_eq!(
            status(
                put(
                    &tank,
                    serde_json::json!({ "target_c": 50.0, "by_minute": 1440 })
                )
                .await
            ),
            Some(StatusCode::BAD_REQUEST)
        );
        // A temperature is not a goal for a car, nor a state of charge for a tank.
        assert_eq!(
            status(
                put(
                    &car,
                    serde_json::json!({ "target_c": 50.0, "by_minute": 420 })
                )
                .await
            ),
            Some(StatusCode::UNPROCESSABLE_ENTITY)
        );
        assert_eq!(
            status(
                put(
                    &tank,
                    serde_json::json!({ "target_soc": 0.8, "by_minute": 420 })
                )
                .await
            ),
            Some(StatusCode::UNPROCESSABLE_ENTITY)
        );
        assert_eq!(
            status(
                put(
                    "nothing",
                    serde_json::json!({ "target_soc": 0.8, "by_minute": 420 })
                )
                .await
            ),
            Some(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            local.home.goal.get(),
            crate::home::GoalSet::default(),
            "a refused goal changes nothing"
        );
    }

    #[tokio::test]
    async fn a_tank_goal_is_kept_beside_the_cars_and_withdrawn_alone() {
        let (local, car, tank) = with_the_reference_house();
        let water = crate::home::TankGoal {
            target_c: 50.0,
            by_minute: 1080,
        };
        let Json(answer) = set_goal(
            State(local.clone()),
            None,
            Path(tank.clone()),
            Json(serde_json::json!(water)),
        )
        .await
        .expect("a tank takes a temperature");
        assert!(
            answer.is_null(),
            "no verdict before the planner has seen the tank: {answer}"
        );
        let Json(_) = set_goal(
            State(local.clone()),
            None,
            Path(car.clone()),
            Json(serde_json::json!({ "target_soc": 0.8, "by_minute": 420 })),
        )
        .await
        .expect("a car takes a state of charge");
        let set = local.home.goal.get();
        assert_eq!(set.tank, Some(water));
        assert!(set.car.is_some());
        let Json(document) = home(State(local.clone()))
            .await
            .expect("read back saved goals");
        assert_eq!(document.tank_goal, Some((water, None)));
        assert_eq!(document.goal, set.car.map(|goal| (goal, None)));

        assert_eq!(
            clear_goal(State(local.clone()), None, Path(car.clone())).await,
            Ok(StatusCode::NO_CONTENT)
        );
        assert_eq!(
            clear_goal(State(local.clone()), None, Path(car)).await,
            Ok(StatusCode::NOT_FOUND),
            "there is no car goal left to withdraw"
        );
        assert_eq!(local.home.goal.get().tank, Some(water));
    }
}
