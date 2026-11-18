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
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};

use crate::home::{self, CarGoal, GoalAnswer, Home};
use crate::scenario::{DayResult, Recording, Scenario};

/// The SKI the demo shows. Not any device's: a demo pairs with nothing.
const DEMO_SKI: &str = "3f2a9c4107bed5e81a6f44c2b9037d18e6a502fc";
/// The fingerprint the demo shows.
const DEMO_FINGERPRINT: &str = "8e41c3a90b7d2f6614e8a0c5d2b97f31a4e6c0d58b2f19e7a3c6d40e5b81c702";

/// A replay the page can be served from.
#[derive(Clone)]
pub struct Demo {
    recording: Arc<Recording>,
    receipt: home::Receipt,
    goal: Arc<tokio::sync::Mutex<Option<CarGoal>>>,
}

impl Demo {
    /// Simulate `scenario` once and keep every minute of it.
    ///
    /// # Errors
    /// Where the day cannot be simulated.
    pub fn record(scenario: &Scenario) -> anyhow::Result<Self> {
        let (day, recording) = crate::scenario::run_recorded(scenario)?;
        Ok(Self {
            receipt: receipt_of_day(&day, scenario.date),
            recording: Arc::new(recording),
            goal: Arc::default(),
        })
    }

    /// The page's document at `minute` of the day.
    #[must_use]
    pub fn home_at(&self, minute: usize, goal: Option<CarGoal>) -> Home {
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
                goal,
                car: frame.car,
                ski: Some(DEMO_SKI.into()),
                fingerprint: Some(DEMO_FINGERPRINT.into()),
                https_fingerprint: None,
            },
            now,
        );
        view.demo = true;
        view.at_minute = u16::try_from(index).ok();
        view.receipt = Some(self.receipt.clone());
        view
    }

    /// The routes: the page, its document, and the few writes it makes.
    pub fn router(self) -> axum::Router {
        axum::Router::new()
            .route("/v1/home", get(home_route))
            .route("/v1/goals/car", put(set_goal).delete(clear_goal))
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

/// A finished simulated day, as the receipt the page prints.
#[must_use]
pub fn receipt_of_day(day: &DayResult, date: time::Date) -> home::Receipt {
    home::Receipt {
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

async fn set_goal(
    State(demo): State<Demo>,
    Json(goal): Json<CarGoal>,
) -> Result<Json<Option<GoalAnswer>>, StatusCode> {
    if !(0.0..=1.0).contains(&goal.target_soc) || goal.by_minute >= 1440 {
        return Err(StatusCode::BAD_REQUEST);
    }
    *demo.goal.lock().await = Some(goal);
    Ok(Json(None))
}

async fn clear_goal(State(demo): State<Demo>) -> StatusCode {
    *demo.goal.lock().await = None;
    StatusCode::NO_CONTENT
}

async fn pairing() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "ski": DEMO_SKI, "trusted": ["7b10e3d2a4c95f8812d0b6e3c1a47f90d25ec94f"], "waiting": [] }),
    )
}

/// Serve the demo on `listen` until interrupted.
///
/// # Errors
/// Where `listen` is not a loopback address, or the port cannot be bound.
pub async fn serve(demo: Demo, listen: SocketAddr) -> anyhow::Result<()> {
    anyhow::ensure!(
        listen.ip().is_loopback(),
        "the demo asks for no credential, so it listens only on this machine — not {listen}"
    );
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
        let r = demo.home_at(0, None).receipt.expect("a receipt");
        assert!((r.cost_eur - day.cost.total()).abs() < 1e-9);
        assert!((r.saved_eur - day.saving_eur()).abs() < 1e-9);
        assert_eq!(r.reduced_minutes, Some(day.limited_minutes));
    }

    #[test]
    fn every_minute_of_the_demo_explains_every_device() {
        let scenario = Scenario::winter_with_grid_event(crate::HouseholdConfig::default());
        let demo = Demo::record(&scenario).expect("the winter day");
        for minute in (0..1440).step_by(15) {
            let view = demo.home_at(minute, None);
            assert_ne!(view.devices, []);
            for d in &view.devices {
                assert!(
                    home::ALL_KEYS.contains(&d.why.key),
                    "{minute}: {}",
                    d.why.key
                );
            }
        }
        // The § 14a reduction of the winter day, 17:00–18:30 local, is visible.
        let reduced = demo.home_at(17 * 60 + 30, None);
        assert!(
            reduced.reduction.is_some(),
            "the reduction at 17:30 is on the page"
        );
    }
}
