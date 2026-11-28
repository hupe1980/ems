//! Read-only operator projection and site-authenticated simulator receipts.
use super::Fleet;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum Observation {
    Verified {
        run: String,
        initial_boot: String,
        candidate: String,
        verdict: Verdict,
    },
    Running {
        run: String,
        sent_at: i64,
        boot: String,
        digest: String,
        ready: bool,
        rollback: bool,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Verdict {
    Verified,
    WrongTarget,
    Rejected,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub(super) async fn observe(
    State(fleet): State<Fleet>,
    Path(run): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Observation>,
) -> Result<StatusCode, StatusCode> {
    let (Observation::Verified { run: bound_run, .. }
    | Observation::Running { run: bound_run, .. }) = &input;
    if *bound_run != run {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let token = ems_service::auth::bearer(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
    .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut client = fleet
        .store
        .db()
        .get()
        .await
        .map_err(super::desk::unavailable)?;
    let tx = client
        .transaction()
        .await
        .map_err(super::desk::unavailable)?;
    let row = tx
        .query_opt(
            "SELECT r.candidate, r.initial_boot, r.verdict, r.boot, r.running, r.ready, r.rollback, floor(extract(epoch FROM statement_timestamp()))::bigint, r.sent_at
         FROM rehearsal r JOIN enrolment e ON r.canary = e.site
         WHERE r.id = $1 AND e.token = $2 FOR UPDATE OF r",
            &[&run, &token],
        )
        .await
        .map_err(super::desk::unavailable)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let conflict = record(&tx, &run, &row, input).await?;
    if conflict {
        tx.execute("UPDATE rehearsal SET conflict=TRUE WHERE id=$1", &[&run])
            .await
            .map_err(super::desk::unavailable)?;
    }
    tx.commit().await.map_err(super::desk::unavailable)?;
    Ok(if conflict {
        StatusCode::CONFLICT
    } else {
        StatusCode::NO_CONTENT
    })
}

async fn record(
    tx: &tokio_postgres::Transaction<'_>,
    run: &str,
    row: &tokio_postgres::Row,
    input: Observation,
) -> Result<bool, StatusCode> {
    let old_boot: Option<String> = row.get(1);
    Ok(match input {
        Observation::Verified {
            run: _,
            initial_boot,
            candidate,
            verdict,
        } => {
            if candidate != row.get::<_, String>(0)
                || initial_boot.is_empty()
                || initial_boot.len() > 128
            {
                return Err(StatusCode::UNPROCESSABLE_ENTITY);
            }
            let verdict = match verdict {
                Verdict::Verified => "verified",
                Verdict::WrongTarget => "wrong-target",
                Verdict::Rejected => "rejected",
            };
            if let Some(old) = old_boot {
                old != initial_boot || row.get::<_, String>(2) != verdict
            } else {
                tx.execute(
                    "UPDATE rehearsal SET initial_boot=$2,verdict=$3 WHERE id=$1",
                    &[&run, &initial_boot, &verdict],
                )
                .await
                .map_err(super::desk::unavailable)?;
                false
            }
        }
        Observation::Running {
            run: _,
            sent_at,
            boot,
            digest,
            ready,
            rollback,
        } => {
            let now: i64 = row.get(7);
            if !(now - 120..=now).contains(&sent_at)
                || old_boot.is_none()
                || old_boot.as_ref() == Some(&boot)
                || boot.is_empty()
                || boot.len() > 128
                || !valid_digest(&digest)
                || row.get::<_, Option<String>>(2).as_deref() != Some("verified")
            {
                return Err(StatusCode::UNPROCESSABLE_ENTITY);
            }
            if let Some(old) = row.get::<_, Option<String>>(3) {
                old != boot
                    || row.get::<_, String>(4) != digest
                    || row.get::<_, bool>(5) != ready
                    || row.get::<_, bool>(6) != rollback
                    || row.get::<_, Option<i64>>(8) != Some(sent_at)
            } else {
                tx.execute(
                    "UPDATE rehearsal SET boot=$2, running=$3, ready=$4, rollback=$5,
                            sent_at=$6, observed_at=clock_timestamp() WHERE id=$1",
                    &[&run, &boot, &digest, &ready, &rollback, &sent_at],
                )
                .await
                .map_err(super::desk::unavailable)?;
                false
            }
        }
    })
}

#[derive(Serialize)]
pub(super) struct Review {
    run: String,
    site: String,
    component: String,
    candidate: String,
    cohort: i64,
    verified: bool,
    running: Option<String>,
    ready: Option<bool>,
    observed_at: Option<i64>,
    decision: &'static str,
}

#[derive(Deserialize)]
pub(super) struct Filter {
    run: Option<String>,
}

pub(super) async fn latest(
    Query(filter): Query<Filter>,
    State(fleet): State<Fleet>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse, StatusCode> {
    let scope = super::desk::read_scope(&fleet, &headers)?;
    let client = fleet
        .store
        .db()
        .get()
        .await
        .map_err(super::desk::unavailable)?;
    let row = client.query_opt(
        "SELECT id, canary, component, candidate,
                (SELECT count(*) FROM unnest(cohort) AS s WHERE $1::text[] IS NULL OR s = ANY($1)),
                verdict, running, ready, rollback, conflict,
                floor(extract(epoch FROM observed_at))::bigint,
                observed_at >= statement_timestamp() - interval '120 seconds'
         FROM rehearsal WHERE ($1::text[] IS NULL OR canary = ANY($1)) AND ($2::text IS NULL OR id = $2)
         ORDER BY created_at DESC, id DESC LIMIT 1",
        &[&scope,&filter.run]
    ).await.map_err(super::desk::unavailable)?;
    let review = row.map(|r| {
        let candidate: String = r.get(3);
        let verdict: Option<String> = r.get(5);
        let running: Option<String> = r.get(6);
        let ready: Option<bool> = r.get(7);
        let decision = match (verdict.as_deref(), running.as_ref()) {
            _ if r.get::<_, bool>(9) => "Hold · conflicting observations",
            (None, _) => "Hold · awaiting verification",
            (Some("wrong-target"), _) => "Hold · wrong target",
            (Some("rejected"), _) => "Hold · verification rejected",
            (_, None) => "Hold · awaiting restart",
            _ if r.get::<_, Option<bool>>(8) == Some(true) => "Hold · rolled back",
            (_, Some(build)) if build != &candidate => "Hold · different build running",
            _ if r.get::<_, Option<bool>>(11) != Some(true) => "Hold · stale readiness",
            _ if ready != Some(true) => "Hold · gateway unready",
            _ => "Qualified for next simulated site",
        };
        Review {
            run: r.get(0),
            site: r.get(1),
            component: r.get(2),
            cohort: r.get(4),
            verified: verdict.as_deref() == Some("verified"),
            candidate,
            running,
            ready,
            observed_at: r.get(10),
            decision,
        }
    });
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(review)))
}
