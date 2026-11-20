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
    /// What only the household page reads: the site, the plan, the goal.
    home: HomeSources,
    /// What the configuration says about this box's devices, for the handover
    /// sheet and discovery.
    commissioning: crate::runtime::handover::Commissioning,
    /// The store the closed days are read from, where the box keeps one.
    days: Option<Arc<Mutex<crate::store::Store>>>,
    /// The Web Push sender, where the box has a store to keep subscriptions in.
    push: Option<crate::runtime::push::Push>,
}

/// What the household page needs beyond `/v1/status`.
#[derive(Clone, Default)]
pub struct HomeSources {
    /// The house, so the page can name its devices.
    pub site: Option<Arc<hems_core::prelude::Site>>,
    /// The plan in force.
    pub plan: Arc<tokio::sync::RwLock<Option<hems_core::prelude::Plan>>>,
    /// The prices it was made against.
    pub prices: Arc<tokio::sync::RwLock<Option<hems_tariff::PriceStack>>>,
    /// The household's goals, shared with the planner.
    pub goal: crate::home::Goals,
    /// The car as the planner last saw it.
    pub car: crate::home::CarFacts,
    /// The tank as the planner last saw it.
    pub tank: crate::home::TankFacts,
    /// The SHA-256 fingerprint of the box's SHIP certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate it serves HTTPS with.
    pub https_fingerprint: Option<String>,
}

impl Local {
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

    /// Answer `/v1/push` with this sender.
    #[must_use]
    pub fn with_push(mut self, push: Option<crate::runtime::push::Push>) -> Self {
        self.push = push;
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
        let goals = self.home.goal.get().await;
        let car = *self.home.car.lock().await;
        let tank = *self.home.tank.lock().await;
        Some(crate::home::live(
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
        ))
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
            home: HomeSources::default(),
            commissioning: crate::runtime::handover::Commissioning::default(),
            days: None,
            push: None,
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
    pub roof_health: Option<hems_forecast::Health>,
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
        .route("/v1/qr/{what}", get(qr))
        .route("/v1/handover", get(handover))
        .route("/v1/commissioning", get(commissioning))
        .route("/v1/discover", axum::routing::post(discover))
        .route("/v1/trust", get(trust_check))
        .route("/v1/token", axum::routing::post(replace_token))
        .route("/v1/days", get(days))
        .route("/v1/days.csv", get(days_csv))
        .route("/v1/goals/{asset}", put(set_goal).delete(clear_goal))
        .route(
            "/v1/push",
            get(push_overview).post(push_subscribe).delete(push_forget),
        )
        .route("/v1/push/test", axum::routing::post(push_test))
        .with_state(local)
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
) -> Result<axum::Json<crate::series::Window>, StatusCode> {
    let Some(series) = local.series.as_ref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let now = time::OffsetDateTime::now_utc();
    let to = window.to.unwrap_or(now);
    let from = window.from.unwrap_or(to - time::Duration::hours(1));
    if to <= from {
        return Err(StatusCode::BAD_REQUEST);
    }
    series
        .window(&point, from, to)
        .map(axum::Json)
        .map_err(|error| {
            tracing::warn!(%error, %point, "the measurement series could not be read");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// What the household is asking for, and until when.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct OverrideBody {
    /// `boost`, `pause` or `away`.
    #[schemars(schema_with = "crate::openapi::user_override")]
    pub what: hems_core::setpoint::UserOverride,
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
    pub what: hems_core::setpoint::UserOverride,
    /// When it stops applying, RFC 3339.
    pub until: String,
}

/// Ask for one.
async fn set_override(
    State(local): State<Local>,
    who: Option<axum::Extension<hems_service::auth::Authority>>,
    axum::extract::Path(asset): axum::extract::Path<String>,
    Json(body): Json<OverrideBody>,
) -> Result<Json<OverrideView>, axum::http::StatusCode> {
    let asset = hems_core::prelude::AssetId::new(&asset)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
    axum::extract::Path(asset): axum::extract::Path<String>,
) -> axum::http::StatusCode {
    let Ok(asset) = hems_core::prelude::AssetId::new(&asset) else {
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
) -> axum::http::StatusCode {
    let n = local.overrides.clear_all().await;
    tracing::info!(withdrawn = n, subject = %subject(who.as_deref()), "every household override was withdrawn");
    axum::http::StatusCode::NO_CONTENT
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
fn subject(who: Option<&hems_service::auth::Authority>) -> String {
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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

/// Set a goal for a charge point or a hot-water tank, and answer at once
/// whether it can be met.
///
/// The body follows the asset: `target_soc` and `by_minute` for a car,
/// `target_c` and `by_minute` for a tank. The answer is the verdict, or `null`
/// where the planner has not seen the device yet.
async fn set_goal(
    State(local): State<Local>,
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
        NewGoal::Car(g) => local.home.car.lock().await.map(|facts| {
            serde_json::json!(crate::home::judge_goal(
                &crate::home::session_for(&facts, g, now),
                hems_core::prelude::Slot::containing(now),
            ))
        }),
        NewGoal::Tank(g) => {
            let tank = site.assets.iter().find_map(|a| match a {
                hems_core::prelude::Asset::Dhw(tank) => Some(tank),
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
        steuve_ceiling_kw: held.steuve_ceiling.map(hems_core::prelude::Power::kw),
        steuve_budget_kw: held.steuve_budget.map(hems_core::prelude::Power::kw),
        netzwirksam_kw: held.netzwirksam.map(hems_core::prelude::Power::kw),
        balance_residual_w: held.balance_residual.map(hems_core::prelude::Power::get),
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
    axum::extract::Path(ski): axum::extract::Path<String>,
) -> Result<Json<PairingBody>, (axum::http::StatusCode, String)> {
    let Some(trust) = local.trust.as_ref() else {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "this box has no EEBUS identity, so there is nothing to pair with".into(),
        ));
    };
    trust
        .forget(&ski, OffsetDateTime::now_utc())
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
    tracing::info!(?ski, subject = %subject(who.as_deref()), "a control box was forgotten");
    Ok(list_trusted(State(local)).await)
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
    Ok(Json(
        handover::discover(
            &local.commissioning,
            &known,
            &hosts,
            ask.unit.unwrap_or(1),
            ask.port.unwrap_or(handover::MODBUS_PORTS[0]),
        )
        .await,
    ))
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
    who: Option<axum::Extension<hems_service::auth::Authority>>,
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
async fn trust_check(State(local): State<Local>) -> Json<crate::runtime::handover::TrustCheck> {
    Json(trust_of(&local).await)
}

/// The trust check, for the route and for the commissioning record.
async fn trust_of(local: &Local) -> crate::runtime::handover::TrustCheck {
    use crate::runtime::handover;
    let identity = match &local.trust {
        None => "none",
        Some(trust) if trust.identity_is_kept().await => "kept",
        Some(_) => "ephemeral",
    };
    let (ticked, silent, in_failsafe) = {
        let status = local.status.lock().await;
        let silent = local
            .commissioning
            .control_box
            .as_ref()
            .is_some_and(|asset| status.silent.contains(asset));
        (status.at.is_some(), silent, status.in_failsafe)
    };
    handover::TrustCheck {
        identity,
        ski: local.ski.clone(),
        fingerprint: local.home.fingerprint.clone(),
        https_fingerprint: local.home.https_fingerprint.clone(),
        control_box: handover::control_box(handover::ControlBoxFacts {
            configured: local.trust.is_some() && local.commissioning.control_box.is_some(),
            trusted: local.trust.as_ref().map_or(0, |t| t.peers().len()),
            waiting: local.trust.as_ref().map_or(0, |t| t.waiting().len()),
            // Before the first tick nothing has been heard *or* missed.
            silent: !ticked || silent,
            in_failsafe,
        }),
    }
}

#[cfg(test)]
mod series_tests {
    use super::*;

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
        let outcome = series(
            State(local),
            Path("grid".to_owned()),
            Query(Asked {
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
                &[(crate::series::GRID, hems_core::prelude::Power::from_kw(2.5))],
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
        // No window: the default hour back is what a screen asks for, and it has
        // to be wide enough to contain a reading five minutes old.
        let Json(window) = series(
            State(local),
            Path(crate::series::GRID.to_owned()),
            Query(Asked {
                from: None,
                to: None,
            }),
        )
        .await
        .expect("a box with a series answers");
        assert_eq!(
            window.resolution,
            crate::series::Resolution::Seconds,
            "an hour back is the box's own trace, not a rollup of it"
        );
        assert_eq!(window.readings.len(), 1, "{window:?}");
        assert!((window.readings[0].watts - 2_500.0).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
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
            local.home.goal.get().await,
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
        let set = local.home.goal.get().await;
        assert_eq!(set.tank, Some(water));
        assert!(set.car.is_some());

        assert_eq!(
            clear_goal(State(local.clone()), Path(car.clone())).await,
            Ok(StatusCode::NO_CONTENT)
        );
        assert_eq!(
            clear_goal(State(local.clone()), Path(car)).await,
            Ok(StatusCode::NOT_FOUND),
            "there is no car goal left to withdraw"
        );
        assert_eq!(local.home.goal.get().await.tank, Some(water));
    }
}
