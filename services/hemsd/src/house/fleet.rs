//! The house's own day, served as the fleet serves it: prices as `tariffd`
//! answers them and a sky as `forecastd` does.
//!
//! Without them the box that manages the house has nothing to plan against and
//! says so — *cannot forecast its own roof and will not plan* — so the real
//! product in one command would be the product without its planner. The house
//! knows its day: the reference day's 96 day-ahead prices and the weather the
//! simulator draws its sky from.
//!
//! The box lives on the wall clock and the house on its reference day, so every
//! slot the box asks about is mapped to the same moment of the house's day
//! (`Clock::scenario_at`); a horizon that runs past midnight wraps onto the
//! same day again, which is what a reference day is.
//!
//! The sky is a **forecast**, not the answer: the clear-sky irradiance with the
//! day's expected cloud and the diurnal temperature, never the realisation the
//! house's roof then produces. A house that told its box the weather would be
//! the oracle the evaluation was built to refuse.

use std::net::SocketAddr;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use hems_core::prelude::Slot;
use time::{Duration, OffsetDateTime};

use super::Clock;
use crate::forecasting::Weather;
use crate::scenario::Scenario;

/// What the endpoints read.
#[derive(Clone)]
struct Day {
    clock: Clock,
    midnight: OffsetDateTime,
    prices_ct: Vec<i64>,
    weather: Weather,
    location: hems_core::prelude::GeoPoint,
}

/// The location name the box's configuration asks `forecastd` for.
pub const LOCATION: &str = "house";

/// How far the sky reaches: the box's whole two-day horizon.
const SKY_SLOTS: i64 = 192;

/// Serve the house's prices and sky on `listen`; the address it bound.
///
/// # Errors
/// The address cannot be bound.
pub async fn serve(
    listen: SocketAddr,
    scenario: &Scenario,
    clock: Clock,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let day = Day {
        clock,
        midnight: scenario.start(),
        prices_ct: scenario.prices_ct.clone(),
        weather: Weather::new(scenario.weather, scenario.cloudiness, scenario.outdoor_c),
        location: scenario.config.location,
    };
    let router = axum::Router::new()
        .route("/v1/prices", get(prices))
        .route("/v1/weather/{location}", get(weather))
        .with_state(day);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let bound = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((bound, task))
}

#[derive(serde::Deserialize)]
struct Window {
    #[serde(with = "time::serde::rfc3339")]
    from: OffsetDateTime,
    slots: u16,
}

/// `GET /v1/prices?from=…&slots=…`, in `tariffd`'s shape.
async fn prices(
    State(day): State<Day>,
    Query(window): Query<Window>,
) -> axum::Json<serde_json::Value> {
    let first = Slot::containing(window.from);
    let points: Vec<serde_json::Value> = (0..i64::from(window.slots))
        .map(|i| {
            let start = first.start() + Duration::minutes(15 * i);
            serde_json::json!({
                "slot": rfc3339(start),
                "price_ct": day.price_at(start).to_string(),
                "source": hems_tariff::source::Source::EnergyCharts,
            })
        })
        .collect();
    let until = first.start() + Duration::minutes(15 * i64::from(window.slots));
    axum::Json(serde_json::json!({
        "points": points,
        "coverage": 1.0,
        "contiguous_until": rfc3339(until),
    }))
}

/// `GET /v1/weather/{location}`, in `forecastd`'s shape.
async fn weather(State(day): State<Day>, Path(location): Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if location != LOCATION {
        return (
            axum::http::StatusCode::NOT_FOUND,
            "the house is the only location",
        )
            .into_response();
    }
    let now = OffsetDateTime::now_utc();
    let first = Slot::containing(now).start();
    let points: Vec<serde_json::Value> = (0..SKY_SLOTS)
        .map(|i| {
            let start = first + Duration::minutes(15 * i);
            let lived = day.clock.scenario_at(start);
            let sun = hems_forecast::solar::sun_position(day.location, Slot::containing(lived));
            let clear = hems_forecast::solar::clear_sky_ghi(sun);
            serde_json::json!({
                "slot": rfc3339(start),
                "ghi_w_per_m2": clear * (1.0 - day.weather.mean_cloud),
                "temperature_c": day.weather.forecast_outdoor_at(lived),
                "cloud_cover": day.weather.mean_cloud,
            })
        })
        .collect();
    axum::Json(serde_json::json!({
        "fetched_at": rfc3339(now),
        "published_minutes": 15,
        "points": points,
    }))
    .into_response()
}

impl Day {
    /// The reference day's price in the slot the house lives at `real`.
    fn price_at(&self, real: OffsetDateTime) -> i64 {
        let lived = self.clock.scenario_at(real);
        let slot = (lived - self.midnight).whole_minutes().div_euclid(15);
        let n = i64::try_from(self.prices_ct.len()).unwrap_or(1).max(1);
        let index = usize::try_from(slot.rem_euclid(n)).unwrap_or(0);
        self.prices_ct.get(index).copied().unwrap_or(0)
    }
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
