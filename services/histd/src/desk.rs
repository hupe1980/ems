//! Operator projection of the closed quarters in a bounded Berlin calendar window.
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use ems_core::calendar::{FIRST_YEAR, day_end_utc, day_start_utc, local_day};
use ems_core::prelude::Slot;
use serde::{Deserialize, Serialize};
use time::{Date, Duration, OffsetDateTime};

use super::{History, failed};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DayQuery {
    site: String,
    #[serde(with = "ems_core::wire::iso_date")]
    day: Date,
    #[serde(default = "one_day")]
    days: u8,
}

fn one_day() -> u8 {
    1
}

#[derive(Serialize)]
struct Interval {
    start_unix_s: i64,
    import_kwh: String,
    export_kwh: String,
}

pub(super) async fn energy(
    State(history): State<History>,
    headers: HeaderMap,
    Query(query): Query<DayQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let caller = history.authority(&headers)?;
    if !caller.may_read_the_fleet() || !caller.may_read(&query.site) {
        return Err(StatusCode::FORBIDDEN);
    }
    let now = OffsetDateTime::now_utc();
    if query.site.is_empty() || !(1..=90).contains(&query.days) || query.day > local_day(now) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let first = query
        .day
        .checked_sub(Duration::days(i64::from(query.days) - 1))
        .ok_or(StatusCode::BAD_REQUEST)?;
    if first.year() < FIRST_YEAR {
        return Err(StatusCode::BAD_REQUEST);
    }
    let start = day_start_utc(first);
    let end = day_end_utc(query.day);
    let closed = Slot::containing(now).start().min(end);
    let intervals: Vec<_> = failed(
        history
            .store
            .quarter_hours(&query.site, Some(start), Some(closed))
            .await,
    )?
    .into_iter()
    .map(|q| Interval {
        start_unix_s: q.slot.start().unix_timestamp(),
        import_kwh: q.grid_draw.normalize().to_string(),
        export_kwh: q.grid_feed_in.normalize().to_string(),
    })
    .collect();
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        axum::Json(serde_json::json!({
            "site": query.site, "day": query.day.to_string(), "days": query.days, "first_day": first.to_string(), "start_unix_s": start.unix_timestamp(),
            "end_unix_s": end.unix_timestamp(), "closed_until_unix_s": closed.unix_timestamp(), "intervals": intervals,
        })),
    ))
}
