//! Gateway evidence received by the fleet.
use super::Fleet;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

#[expect(
    clippy::struct_field_names,
    reason = "Wire timestamps carry the _at suffix."
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkin {
    sent_at: i64,
    feeder_at: Option<i64>,
    report_at: Option<i64>,
}

#[expect(
    clippy::struct_field_names,
    reason = "Wire timestamps carry the _at suffix."
)]
#[derive(Serialize)]
struct Receipt {
    sent_at: i64,
    received_at: i64,
    feeder_at: Option<i64>,
    report_at: Option<i64>,
}

pub(super) fn unavailable(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(%error, "fleet desk storage unavailable");
    StatusCode::SERVICE_UNAVAILABLE
}

pub(super) async fn checkin(
    State(fleet): State<Fleet>,
    headers: HeaderMap,
    Json(input): Json<Checkin>,
) -> Result<StatusCode, StatusCode> {
    let token = ems_service::auth::bearer(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok()),
    )
    .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut client = fleet.store.db().get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let site: String = tx
        .query_opt(
            "SELECT site FROM enrolment WHERE token=$1 FOR UPDATE",
            &[&token],
        )
        .await
        .map_err(unavailable)?
        .ok_or(StatusCode::UNAUTHORIZED)?
        .get(0);
    let now: i64 = tx
        .query_one(
            "SELECT floor(extract(epoch from clock_timestamp()))::bigint",
            &[],
        )
        .await
        .map_err(unavailable)?
        .get(0);
    if !(now - 90..=now).contains(&input.sent_at)
        || [input.feeder_at, input.report_at]
            .into_iter()
            .flatten()
            .any(|v| !(0..=input.sent_at).contains(&v))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let inserted = tx
        .execute(
            "INSERT INTO gateway_checkin (site, sent_at, received_at, feeder_at, report_at)
             SELECT $1, $2, $3, $4, $5 WHERE NOT EXISTS (
                 SELECT 1 FROM gateway_checkin WHERE site = $1 AND sent_at >= $2
             )",
            &[
                &site,
                &input.sent_at,
                &now,
                &input.feeder_at,
                &input.report_at,
            ],
        )
        .await
        .map_err(unavailable)?;
    if inserted != 0 {
        tx.execute(
            "DELETE FROM gateway_checkin WHERE site = $1 AND sent_at <= (
                 SELECT sent_at FROM gateway_checkin WHERE site = $1
                 ORDER BY sent_at DESC OFFSET 32 LIMIT 1
             )",
            &[&site],
        )
        .await
        .map_err(unavailable)?;
    }
    tx.commit().await.map_err(unavailable)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Signal {
    state: &'static str,
    age_s: Option<i64>,
    stale_after_s: i64,
}

impl Signal {
    fn at(now: i64, sample: Option<i64>, limit: i64) -> Self {
        let age_s = sample.map(|v| now.saturating_sub(v));
        let state = match age_s {
            None => "unknown",
            Some(age) if age < 0 => "clock error",
            Some(age) if age > limit => "stale",
            Some(_) => "current",
        };
        Self {
            state,
            age_s,
            stale_after_s: limit,
        }
    }
}

#[derive(Serialize)]
pub(super) struct DeskRow {
    site: String,
    gateway: Signal,
    feeder: Signal,
    report: Signal,
    attention: Attention,
    history: Vec<Receipt>,
}

// Queue priorities are assigned in desk_page.sql.
#[derive(Clone, Copy, Serialize)]
enum Attention {
    #[serde(rename = "Gateway silent")]
    Silent,
    #[serde(rename = "Check fleet clock")]
    Clock,
    #[serde(rename = "Inspect feeder")]
    Feeder,
    #[serde(rename = "Inspect closed report")]
    Report,
    #[serde(rename = "Awaiting first check-in")]
    First,
    Current,
}

const PAGE_SIZE: i64 = 15;
const GATEWAY_AGE: i64 = 90;
const FEEDER_AGE: i64 = 120;
const REPORT_AGE: i64 = 36 * 3600;

#[derive(Serialize)]
struct DeskPage {
    total: i64,
    attention: i64,
    matched: i64,
    page: i64,
    page_size: i64,
    rows: Vec<DeskRow>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct DeskQuery {
    site: Option<String>,
    q: String,
    filter: Option<String>,
    run: Option<String>,
    page: u32,
}

pub(super) fn read_scope(
    fleet: &Fleet,
    headers: &HeaderMap,
) -> Result<Option<Vec<String>>, StatusCode> {
    let authority = fleet
        .operators
        .authority_in(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok()),
        )
        .filter(ems_service::Authority::may_read_the_fleet)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !authority
        .capabilities()
        .permits(ems_service::auth::RECORD_READ)
    {
        return Ok(Some(Vec::new()));
    }
    Ok(authority
        .sites()
        .named()
        .map(|sites| sites.into_iter().map(str::to_owned).collect()))
}

pub(super) async fn queue(
    Query(query): Query<DeskQuery>,
    State(fleet): State<Fleet>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let scope = read_scope(&fleet, &headers)?;
    let filter = query.filter.as_deref().unwrap_or("all");
    if !matches!(filter, "all" | "attention" | "current" | "cohort")
        || query.q.len() > 512
        || (filter == "cohort" && query.run.is_none())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let client = fleet.store.db().get().await.map_err(unavailable)?;
    let result = client
        .query(
            include_str!("desk_page.sql"),
            &[
                &scope,
                &query.site,
                &query.q,
                &filter,
                &query.run,
                &i64::from(query.page),
                &GATEWAY_AGE,
                &FEEDER_AGE,
                &REPORT_AGE,
                &PAGE_SIZE,
            ],
        )
        .await
        .map_err(unavailable)?;
    let counts = &result[0];
    let mut page = DeskPage {
        total: counts.get("total"),
        attention: counts.get("attention"),
        matched: counts.get("matched"),
        page: counts.get("page"),
        page_size: PAGE_SIZE,
        rows: Vec::with_capacity(result.len()),
    };
    for row in &result {
        let Some(site) = row.get::<_, Option<String>>("site") else {
            continue;
        };
        if page
            .rows
            .last()
            .is_none_or(|previous| previous.site != site)
        {
            let now = row.get("now_s");
            let attention = match row.get::<_, i32>("priority") {
                0 => Attention::Silent,
                1 => Attention::Clock,
                2 => Attention::Feeder,
                3 => Attention::Report,
                4 => Attention::First,
                _ => Attention::Current,
            };
            page.rows.push(DeskRow {
                site,
                attention,
                gateway: Signal::at(now, row.get("received_at"), GATEWAY_AGE),
                feeder: Signal::at(now, row.get("feeder_at"), FEEDER_AGE),
                report: Signal::at(now, row.get("report_at"), REPORT_AGE),
                history: Vec::new(),
            });
        }
        if let (Some(sent_at), Some(site)) = (row.get("sent_at"), page.rows.last_mut()) {
            site.history.push(Receipt {
                sent_at,
                received_at: row.get("receipt_at"),
                feeder_at: row.get("sample_at"),
                report_at: row.get("closed_at"),
            });
        }
    }
    let headers = [(header::CACHE_CONTROL, "no-store")];
    if query.site.is_some() {
        return Ok((headers, Json(page.rows)).into_response());
    }
    Ok((headers, Json(page)).into_response())
}

pub(super) async fn page() -> Html<&'static str> {
    static PAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    Html(
        PAGE.get_or_init(|| {
            include_str!("desk.html")
                .replace(
                    "/* compiled desk styles */",
                    include_str!("assets/desk.css"),
                )
                .replace(
                    "/* compiled meter history */",
                    include_str!("assets/history.js"),
                )
        })
        .as_str(),
    )
}
