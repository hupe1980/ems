//! The latest simulated household departure, scoped by its viewer's credential.
use super::{Observed, verify};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use ems_events::departure::SimulatedDeparture;
use time::OffsetDateTime;

fn unavailable(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(%error,"departure receipt store unavailable");
    StatusCode::SERVICE_UNAVAILABLE
}

pub(super) async fn receive(
    State(state): State<Observed>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    let now = OffsetDateTime::now_utc();
    let site = verify(&state, &headers, &body, now).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event =
        ems_events::Event::<SimulatedDeparture>::parse(&body, ems_events::SITE_DEPARTURE_REHEARSED)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
    if event.source != format!("ems://sites/{site}") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let receipt = event.data;
    // PostgreSQL stores microseconds. Reject a finer timestamp before comparing identity.
    if !receipt.valid()
        || receipt.departure > now
        || event.time != receipt.departure
        || !receipt.departure.nanosecond().is_multiple_of(1000)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let client = state.store.db().get().await.map_err(unavailable)?;
    let changed = client
        .execute(
            include_str!("store_departure.sql"),
            &[
                &site,
                &receipt.departure,
                &receipt.requested_wh,
                &receipt.opening_wh,
                &receipt.rejoin_wh,
                &receipt.closing_wh,
            ],
        )
        .await
        .map_err(unavailable)?;
    if changed == 0 {
        return Err(StatusCode::CONFLICT);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn read(State(state): State<Observed>, headers: HeaderMap) -> Response {
    let result = customer_receipt(&state, &headers).await;
    ([(header::CACHE_CONTROL, "private, no-store")], result).into_response()
}

async fn customer_receipt(state: &Observed, headers: &HeaderMap) -> Result<Response, StatusCode> {
    let site = customer_site(state, headers)?;
    let client = state.store.db().get().await.map_err(unavailable)?;
    let row = client
        .query_opt(
            "SELECT departure, requested_wh, opening_wh, rejoin_wh, closing_wh
         FROM departure_receipt WHERE site = $1",
            &[&site],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let evidence = SimulatedDeparture {
        departure: row.get(0),
        requested_wh: row.get(1),
        opening_wh: row.get(2),
        rejoin_wh: row.get(3),
        closing_wh: row.get(4),
    };
    let delivered = evidence.closing_wh.map(|v| v - evidence.opening_wh);
    let shortfall = delivered.map(|v| (evidence.requested_wh - v).max(0));
    Ok(Json(serde_json::json!({
        "site": site,
        "simulation": true,
        "evidence": evidence,
        "delivered_wh": delivered,
        "shortfall_wh": shortfall
    }))
    .into_response())
}

fn customer_site(state: &Observed, headers: &HeaderMap) -> Result<String, StatusCode> {
    let authority = state
        .customers
        .authority_in(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
        )
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let ems_service::SiteScope::One(site) = authority.sites() else {
        return Err(StatusCode::FORBIDDEN);
    };
    if !authority.may_read(site) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(site.to_owned())
}

pub(super) async fn history(State(state): State<Observed>, headers: HeaderMap) -> Response {
    let result = async {
        let site = customer_site(&state, &headers)?;
        let client = state.store.db().get().await.map_err(unavailable)?;
        let rows = client
            .query(
                "SELECT jsonb_build_object('day', day, 'imported', document->'imported_kwh',
             'exported', document->'exported_kwh', 'produced', document->'produced_kwh')
             FROM site_day WHERE site=$1 ORDER BY day DESC LIMIT 30",
                &[&site],
            )
            .await
            .map_err(unavailable)?;
        Ok::<_, StatusCode>(Json(
            rows.iter()
                .map(|r| r.get::<_, serde_json::Value>(0))
                .collect::<Vec<_>>(),
        ))
    }
    .await;
    ([(header::CACHE_CONTROL, "private, no-store")], result).into_response()
}
