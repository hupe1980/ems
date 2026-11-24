//! Gateway evidence received by the fleet.
use super::Fleet;
use crate::store::StoreError;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Html,
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

fn unavailable(error: impl std::fmt::Display) -> StatusCode {
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
        Self { state, age_s }
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

// Declaration order is the operator queue's priority.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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

async fn read_sites(
    fleet: &Fleet,
    scope: Option<Vec<String>>,
    site: Option<String>,
) -> Result<Vec<DeskRow>, StoreError> {
    let client = fleet.store.db().get().await?;
    let rows = client
        .query(
            "SELECT e.site,c.sent_at,c.received_at,c.feeder_at,c.report_at,
        floor(extract(epoch from statement_timestamp()))::bigint
        FROM enrolment e LEFT JOIN LATERAL (SELECT * FROM gateway_checkin
            WHERE site=e.site ORDER BY sent_at DESC LIMIT (CASE WHEN $2::text IS NULL THEN 1 ELSE 32 END)) c ON TRUE
        WHERE ($1::text[] IS NULL OR e.site=ANY($1)) AND ($2::text IS NULL OR e.site=$2) ORDER BY e.site,c.sent_at DESC",
            &[&scope, &site],
        )
        .await?;
    let mut sites: Vec<DeskRow> = Vec::new();
    for row in rows {
        let name: String = row.get(0);
        if sites.last().is_none_or(|s| s.site != name) {
            let now = row.get(5);
            let gateway = Signal::at(now, row.get(2), 90);
            let feeder = Signal::at(now, row.get(3), 120);
            let report = Signal::at(now, row.get(4), 36 * 3600);
            let attention = match (gateway.state, feeder.state, report.state) {
                ("unknown", _, _) => Attention::First,
                ("clock error", _, _) => Attention::Clock,
                ("stale", _, _) => Attention::Silent,
                ("current", "current", "current") => Attention::Current,
                ("current", "current", _) => Attention::Report,
                _ => Attention::Feeder,
            };
            sites.push(DeskRow {
                site: name,
                gateway,
                feeder,
                report,
                attention,
                history: Vec::new(),
            });
        }
        if site.is_some()
            && let (Some(sent_at), Some(site)) = (row.get(1), sites.last_mut())
        {
            site.history.push(Receipt {
                sent_at,
                received_at: row.get(2),
                feeder_at: row.get(3),
                report_at: row.get(4),
            });
        }
    }
    sites.sort_by_key(|row| row.attention);
    Ok(sites)
}

#[derive(Deserialize)]
pub(super) struct DeskQuery {
    site: Option<String>,
}

pub(super) async fn queue(
    Query(query): Query<DeskQuery>,
    State(fleet): State<Fleet>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, StatusCode> {
    let authority = fleet
        .operators
        .authority_in(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok()),
        )
        .filter(ems_service::Authority::may_read_the_fleet)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let scope = authority
        .sites()
        .named()
        .map(|s| s.into_iter().map(str::to_owned).collect());
    let sites = if authority
        .capabilities()
        .permits(ems_service::auth::RECORD_READ)
    {
        read_sites(&fleet, scope, query.site)
            .await
            .map_err(unavailable)?
    } else {
        Vec::new()
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(sites)))
}

pub(super) async fn page() -> Html<&'static str> {
    Html(include_str!("desk.html"))
}
