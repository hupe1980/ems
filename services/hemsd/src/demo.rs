//! `hemsd demo`: a reference day, replayed through the household page.
//!
//! The day is simulated once, minute by minute, through the real guard, arbiter
//! and planner ([`crate::scenario::run_recorded`]), and every minute is then
//! served through [`crate::home::live`] — the function the running box uses.
//! The page cannot tell a replay from a box, which is the point: the demo, the
//! screenshots in the documentation and the household's own screen are one
//! artefact, and a figure on any of them is held to the day by the same tests.
//!
//! **Loopback only and no credential.** The demo carries no household's data
//! and serves no device, so it asks for no token — and for exactly that reason
//! it refuses to listen anywhere but on this machine.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};

use crate::home::{self, Home};
use crate::month::ClosedDay;
use crate::scenario::{DayResult, Recording, Scenario};

/// The SKI the demo shows. Not any device's: a demo pairs with nothing.
const DEMO_SKI: &str = "3f2a9c4107bed5e81a6f44c2b9037d18e6a502fc";
/// The fingerprint the demo shows.
const DEMO_FINGERPRINT: &str = "8e41c3a90b7d2f6614e8a0c5d2b97f31a4e6c0d58b2f19e7a3c6d40e5b81c702";

/// A replay the page can be served from.
#[derive(Clone)]
pub struct Demo {
    recording: Arc<Recording>,
    receipt: ClosedDay,
    goal: Arc<tokio::sync::Mutex<home::GoalSet>>,
    /// The closed days before the replayed one, oldest first.
    history: Arc<Vec<ClosedDay>>,
}

impl Demo {
    /// Simulate `scenario` once and keep every minute of it.
    ///
    /// # Errors
    /// Where the day cannot be simulated.
    pub fn record(scenario: &Scenario) -> anyhow::Result<Self> {
        let (day, recording) = crate::scenario::run_recorded(scenario)?;
        Ok(Self {
            receipt: closed_day(&day, scenario.date),
            recording: Arc::new(recording),
            goal: Arc::default(),
            history: Arc::default(),
        })
    }

    /// The same, with the `days` days before it simulated too, so the month
    /// view has closed days to add up.
    ///
    /// Each is a whole day through the same guard, arbiter and planner — the
    /// same household on the dates before the replayed one, under weather that
    /// strays from the reference day's by a fixed, repeatable amount, with the
    /// network operator's reduction on one day a week. Nothing is drawn from a
    /// distribution at start-up: the same command shows the same month.
    ///
    /// # Errors
    /// Where a day cannot be simulated.
    pub fn record_with_history(scenario: &Scenario, days: usize) -> anyhow::Result<Self> {
        let mut demo = Self::record(scenario)?;
        let days = days.min(crate::month::KEPT_DAYS);
        let earlier: Vec<Scenario> = (1..=days).rev().map(|ago| earlier(scenario, ago)).collect();
        // Independent days, so one thread each up to what the machine has.
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let mut history = Vec::with_capacity(days);
        for batch in earlier.chunks(workers) {
            let results: Vec<anyhow::Result<ClosedDay>> = std::thread::scope(|scope| {
                let running: Vec<_> = batch
                    .iter()
                    .map(|day| {
                        scope.spawn(move || {
                            let result = crate::scenario::run(day)?;
                            Ok(closed_day(&result, day.date))
                        })
                    })
                    .collect();
                running
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("a simulated day panicked")))
                    })
                    .collect()
            });
            for day in results {
                history.push(day?);
            }
        }
        demo.history = Arc::new(history);
        Ok(demo)
    }

    /// The page's document at `minute` of the day.
    #[must_use]
    pub fn home_at(&self, minute: usize, goals: home::GoalSet) -> Home {
        let frames = &self.recording.frames;
        let index = minute.min(frames.len().saturating_sub(1));
        let frame = &frames[index];
        let now = frame.status.at.unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let mut view = home::live(
            &home::Live {
                site: &self.recording.site,
                status: &frame.status,
                plan: frame.plan.map(|i| &self.recording.plans[i]),
                prices: Some(&self.recording.prices),
                goals,
                car: frame.car,
                tank: frame.tank,
                ski: Some(DEMO_SKI.into()),
                fingerprint: Some(DEMO_FINGERPRINT.into()),
                https_fingerprint: None,
            },
            now,
        );
        view.demo = true;
        view.thermal_trial = cfg!(feature = "house").then(|| "/thermal-trial".to_owned());
        view.at_minute = u16::try_from(index).ok();
        view.receipt = Some(self.receipt.clone());
        view
    }

    /// The routes: the page, its document, and the few writes it makes.
    pub fn router(self) -> axum::Router {
        let routes = axum::Router::new();
        #[cfg(feature = "house")]
        let routes = routes.route("/thermal-trial", get(thermal_trial));
        routes
            .route("/v1/home", get(home_route))
            .route("/v1/days", get(days))
            .route("/v1/days.csv", get(days_csv))
            .route("/v1/goals/{asset}", put(set_goal).delete(clear_goal))
            .route("/v1/pairing", get(pairing))
            .route(
                "/v1/session",
                post(|| async { StatusCode::NO_CONTENT })
                    .delete(|| async { StatusCode::NO_CONTENT }),
            )
            .with_state(self)
            .merge(crate::ui::router())
    }
}

#[cfg(feature = "house")]
async fn thermal_trial() -> Result<axum::response::Html<&'static str>, StatusCode> {
    static REPORT: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let report = REPORT
        .get_or_try_init(|| async {
            crate::house::winter::trial(2300, 3600)
                .await
                .map(|r| r.html())
        })
        .await
        .map_err(|error| {
            tracing::warn!(%error, "thermal rehearsal unavailable");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    Ok(axum::response::Html(report))
}

/// `scenario` as it would have been `ago` days earlier.
///
/// The weather strays from the reference day's by a repeatable amount — up to
/// ±4 °C and ±0,2 of cloud, on periods that do not divide a week — and the
/// network operator's reduction stays on every seventh day only, which is
/// nearer a household's year than one every evening.
fn earlier(scenario: &Scenario, ago: usize) -> Scenario {
    #[allow(clippy::cast_precision_loss)]
    let n = ago as f64;
    let mut day = scenario.clone();
    day.date = scenario.date - time::Duration::days(i64::try_from(ago).unwrap_or(0));
    day.outdoor_c += 4.0 * (n * 0.7).sin();
    day.cloudiness = (day.cloudiness + 0.2 * (n * 1.3).cos()).clamp(0.0, 0.95);
    if !ago.is_multiple_of(7) {
        day.grid_event = None;
    }
    day
}

fn closed_day(day: &DayResult, date: time::Date) -> ClosedDay {
    let mut receipt = ClosedDay::of(&receipt_of_day(day, date), false);
    if let Some(trial) = &day.service_trial {
        receipt.metered_bill = Some(trial.bill.clone());
        receipt.comparison = trial.comparison;
        receipt.comparison_gap = trial.gap;
    }
    if day.pv_forecast.samples > 0 && day.load_forecast.samples > 0 {
        receipt.forecast = crate::month::ForecastCoverage::of(
            day.pv_forecast.coverage,
            day.load_forecast.coverage,
        );
    }
    receipt
}

/// A finished simulated day, as the receipt the page prints.
#[must_use]
pub fn receipt_of_day(day: &DayResult, date: time::Date) -> home::Receipt {
    home::Receipt {
        priced: true,
        day: date.to_string(),
        bill_eur: day.cost.energy_eur,
        wear_eur: day.cost.wear_eur,
        comfort_eur: day.cost.discomfort_eur,
        cost_eur: day.cost.total(),
        without_eur: day.baseline.total(),
        saved_eur: day.saving_eur(),
        on_bill_eur: day.bill_saving_eur(),
        imported_kwh: day.imported_kwh,
        produced_kwh: day.produced_kwh,
        reduced_minutes: Some(day.limited_minutes),
        reductions: day.control_events,
        reduction_respected: day.grid_event_respected,
        feed_in_respected: day.feed_in_over_minutes == 0,
    }
}

#[derive(serde::Deserialize)]
struct At {
    at: Option<usize>,
}

async fn home_route(State(demo): State<Demo>, Query(at): Query<At>) -> Json<Home> {
    let goal = *demo.goal.lock().await;
    Json(demo.home_at(at.at.unwrap_or(0), goal))
}

async fn days(
    State(demo): State<Demo>,
    Query(window): Query<crate::month::Window>,
) -> Result<Json<crate::month::Days>, StatusCode> {
    window
        .of(demo.history.as_ref().clone())
        .map(Json)
        .ok_or(StatusCode::BAD_REQUEST)
}

async fn days_csv(
    State(demo): State<Demo>,
    Query(window): Query<crate::month::Window>,
) -> Result<axum::response::Response, StatusCode> {
    let found = window
        .of(demo.history.as_ref().clone())
        .ok_or(StatusCode::BAD_REQUEST)?;
    Ok(crate::month::csv_response(&found.days))
}

async fn set_goal(
    State(demo): State<Demo>,
    Path(asset): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let goal = home::parse_goal(&demo.recording.site, &asset, body).map_err(|why| match why {
        home::GoalRefused::UnknownAsset => StatusCode::NOT_FOUND,
        home::GoalRefused::OutOfRange => StatusCode::BAD_REQUEST,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    })?;
    let mut set = demo.goal.lock().await;
    match goal {
        home::NewGoal::Car(g) => set.car = Some(g),
        home::NewGoal::Tank(g) => set.tank = Some(g),
    }
    // The verdict is in the next `/v1/home`, judged at the minute it shows.
    Ok(Json(serde_json::Value::Null))
}

async fn clear_goal(State(demo): State<Demo>, Path(asset): Path<String>) -> StatusCode {
    let mut set = demo.goal.lock().await;
    match home::goal_kind(&demo.recording.site, &asset) {
        Ok("car") => set.car = None,
        Ok(_) => set.tank = None,
        Err(_) => return StatusCode::NOT_FOUND,
    }
    StatusCode::NO_CONTENT
}

async fn pairing() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "ski": DEMO_SKI, "trusted": ["7b10e3d2a4c95f8812d0b6e3c1a47f90d25ec94f"], "waiting": [] }),
    )
}

/// Refuse an address that is not this machine's own.
///
/// Its own function so the command can ask **before** it simulates a day: a
/// refusal that arrives after a minute of simulation is one nobody waits for
/// twice.
///
/// # Errors
/// Where `listen` is not a loopback address.
pub fn loopback_only(listen: SocketAddr) -> anyhow::Result<()> {
    anyhow::ensure!(
        listen.ip().is_loopback(),
        "the demo asks for no credential, so it listens only on this machine — not {listen}"
    );
    Ok(())
}

/// Serve the demo on `listen` until interrupted.
///
/// # Errors
/// Where `listen` is not a loopback address, or the port cannot be bound.
pub async fn serve(demo: Demo, listen: SocketAddr) -> anyhow::Result<()> {
    loopback_only(listen)?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, demo.router()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_receipt_is_the_day_report_to_the_cent() {
        // The receipt the page prints and the transcript the README shows are
        // one day; a test elsewhere holds the transcript to the day, and this
        // holds the receipt to the same arithmetic.
        let scenario = Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
        let demo = Demo::record(&scenario).expect("the winter day");
        let day = crate::scenario::run(&scenario).expect("the winter day");
        let r = demo
            .home_at(0, home::GoalSet::default())
            .receipt
            .expect("a receipt");
        let trial = day.service_trial.unwrap();
        assert_eq!(r.metered_bill.as_ref(), Some(&trial.bill));
        assert_eq!(r.comparison, trial.comparison);
        assert_eq!(r.comparison_gap, trial.gap);
        assert!(r.money.is_none());
        assert_eq!(r.reduced_minutes, Some(day.limited_minutes));
    }

    #[test]
    fn the_demo_month_is_closed_days_before_the_replayed_one() {
        let scenario = Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
        let demo = Demo::record_with_history(&scenario, 2).expect("three winter days");
        let history = demo.history.as_ref();
        // Oldest first, consecutive, priced, and the replayed day itself is
        // not among them — it has not closed.
        let dates: Vec<Option<time::Date>> = history.iter().map(ClosedDay::date).collect();
        assert_eq!(
            dates,
            [
                Some(scenario.date - time::Duration::days(2)),
                scenario.date.previous_day()
            ]
        );
        assert!(history.iter().all(|d| {
            d.metered_bill
                .as_ref()
                .is_some_and(|b| b.energy_ct.is_some())
        }));
        // Different weather is a different day: a month of one bar thirty
        // times would be a picture of nothing.
        assert_ne!(history[0].metered_bill, history[1].metered_bill);
        // No reduction on either; `the_earlier_days_…` says where it is.
        assert!(history.iter().all(|d| d.reductions == 0));
    }

    #[test]
    fn the_earlier_days_vary_the_weather_and_keep_one_reduction_a_week() {
        let scenario = Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
        let days: Vec<Scenario> = (1..=14).map(|ago| earlier(&scenario, ago)).collect();
        let reduced: Vec<usize> = (1..=14)
            .filter(|ago| days[ago - 1].grid_event.is_some())
            .collect();
        assert_eq!(reduced, [7, 14]);
        for day in &days {
            assert!((day.outdoor_c - scenario.outdoor_c).abs() <= 4.0);
            assert!((0.0..=0.95).contains(&day.cloudiness));
        }
        let distinct: std::collections::BTreeSet<i64> = days
            .iter()
            .map(|d| {
                #[allow(clippy::cast_possible_truncation)]
                let tenths = (d.outdoor_c * 10.0).round() as i64;
                tenths
            })
            .collect();
        assert!(distinct.len() > 10, "{distinct:?}");
        // Nothing is drawn at start-up: the same day back is the same day.
        let again = earlier(&scenario, 5);
        assert_eq!(again.date, days[4].date);
        assert!((again.outdoor_c - days[4].outdoor_c).abs() < f64::EPSILON);
        assert!((again.cloudiness - days[4].cloudiness).abs() < f64::EPSILON);
    }

    #[test]
    fn every_minute_of_the_demo_explains_every_device() {
        let config = crate::HouseholdConfig::default();
        for (name, scenario) in [
            ("winter", Scenario::winter_with_grid_event(config.clone())),
            ("summer", Scenario::summer_surplus(config.clone())),
            (
                "deadline",
                Scenario::winter_evening_deadline(config.clone()),
            ),
            ("shared", Scenario::winter_evening_no_store(&config)),
            (
                "offline",
                Scenario::summer_without_a_planner(config.clone()),
            ),
            ("autumn", Scenario::autumn_without_a_planner(config.clone())),
            ("capped", Scenario::summer_capped(&config)),
        ] {
            let demo = Demo::record(&scenario).expect("reference day");
            for minute in (0..1440).step_by(15) {
                let view = demo.home_at(minute, home::GoalSet::default());
                assert!(!view.devices.is_empty(), "{name} {minute}");
                for device in &view.devices {
                    assert!(
                        home::ALL_KEYS.contains(&device.why.key),
                        "{name} {minute}: {} {}",
                        device.id,
                        device.why.key
                    );
                }
                if name == "winter" && minute == 17 * 60 + 30 {
                    assert!(view.reduction.is_some(), "winter reduction at 17:30");
                }
            }
        }
    }
}
