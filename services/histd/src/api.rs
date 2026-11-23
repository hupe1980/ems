//! What a box, a household and a network operator can ask `histd`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use ems_grid::mispel::{QuarterHour, RegisterAssertion};
use ems_service::auth::{Authority, Credentials, bearer};
use time::OffsetDateTime;

use crate::{Store, StoreError};

/// Shared PostgreSQL pool, site credentials and MiSpeL declarations.
#[derive(Clone)]
pub struct History {
    store: Store,
    credentials: Arc<Credentials>,
    mispel: Arc<std::collections::BTreeMap<String, crate::config::MispelSettings>>,
    state_aid_approved_on: Option<time::Date>,
}

impl History {
    /// A handle onto the store, and the credentials it answers to.
    ///
    /// An **empty** [`Credentials`] is a service that answers nothing. What
    /// these routes serve is a household's whole consumption record and the
    /// evidence a network operator settles on, so "nobody configured it" has to
    /// read as "nobody may".
    #[must_use]
    pub fn new(store: Store, credentials: Credentials) -> Self {
        Self {
            store,
            credentials: Arc::new(credentials),
            mispel: Arc::new(std::collections::BTreeMap::new()),
            state_aid_approved_on: None,
        }
    }

    /// Configure each site’s declared MiSpeL option, `[MiSpeL Tenor]`.
    /// The state-aid approval date applies to every site and opens the
    /// Pauschaloption from the following month, `[MiSpeL Tenor 9]` b).
    #[must_use]
    pub fn settling(
        mut self,
        declarations: std::collections::BTreeMap<String, crate::config::MispelSettings>,
        state_aid_approved_on: Option<time::Date>,
    ) -> Self {
        self.mispel = Arc::new(declarations);
        self.state_aid_approved_on = state_aid_approved_on;
        self
    }

    /// What `site` has declared, if anything.
    fn declaration(&self, site: &str) -> Option<crate::config::MispelSettings> {
        self.mispel.get(site).copied()
    }

    /// What the request's bearer token is allowed to do.
    ///
    /// One status for every failure — absent, malformed, unknown. Which it was
    /// is an operational fact for whoever runs the fleet and a probing aid for
    /// anybody else.
    fn authority(&self, headers: &HeaderMap) -> Result<Authority, StatusCode> {
        bearer(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
        )
        .and_then(|token| self.credentials.authority_of(token))
        .ok_or(StatusCode::UNAUTHORIZED)
    }
}

/// Log storage failures once and return HTTP 500.
fn failed<T>(outcome: Result<T, StoreError>) -> Result<T, StatusCode> {
    outcome.map_err(|e| {
        tracing::error!(error = %e, "a query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// The routes.
pub fn router(history: History) -> Router {
    Router::new()
        .route("/v1/sites/{site}/quarter-hours", post(put_quarter_hours))
        .route("/v1/sites/{site}/quarter-hours", get(get_quarter_hours))
        .route("/v1/sites/{site}/events", post(put_event))
        .route("/v1/sites/{site}/nachweis", get(get_nachweis))
        .route("/v1/sites/{site}/export", get(get_export))
        .route("/v1/sites/{site}/mispel", get(get_mispel))
        .with_state(history)
}

/// `?from=<rfc3339>&to=<rfc3339>`.
#[derive(Debug, serde::Deserialize)]
pub struct Window {
    /// Inclusive lower bound.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub from: Option<OffsetDateTime>,
    /// Exclusive upper bound.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub to: Option<OffsetDateTime>,
}

async fn put_quarter_hours(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
    axum::Json(assertions): axum::Json<Vec<RegisterAssertion>>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .authority(&headers)
        .and_then(|caller| deny_unless(caller.may_write(&site)))
        .map_err(|status| (status, String::new()))?;
    state
        .store
        .accept_assertions(&site, &assertions)
        .await
        .map_err(|err| match err {
            StoreError::ConflictingAssertion(_) => (StatusCode::CONFLICT, err.to_string()),
            StoreError::InvalidAssertion(_) => (StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
            other => {
                tracing::error!(error = %other, "register batch failed");
                (StatusCode::INTERNAL_SERVER_ERROR, String::new())
            }
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_quarter_hours(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<axum::Json<Vec<QuarterHour>>, StatusCode> {
    deny_unless(state.authority(&headers)?.may_read(&site))?;
    failed(
        state
            .store
            .quarter_hours(&site, window.from, window.to)
            .await,
    )
    .map(axum::Json)
}

async fn put_event(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
    axum::Json(event): axum::Json<ems_grid::evidence::ControlEvent>,
) -> Result<StatusCode, StatusCode> {
    deny_unless(state.authority(&headers)?.may_write(&site))?;
    failed(state.store.put_control_event(&site, &event).await).map(|_| StatusCode::CREATED)
}

async fn get_nachweis(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<axum::Json<serde_json::Value>, StatusCode> {
    // A network operator may read this: it is the record of what *they*
    // commanded and what the connection point drew, `[A1 7.2]`.
    deny_unless(state.authority(&headers)?.may_read(&site))?;
    failed(crate::export::nachweis(&state.store, &site, window.from, window.to).await)
        .map(axum::Json)
}

async fn get_export(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Result<axum::Json<serde_json::Value>, StatusCode> {
    // Narrower than the Nachweis, and deliberately: Article 4 of Regulation (EU)
    // 2023/2854 is a right of the **user**. This is everything the product
    // generated — when the shower ran, which fortnight nobody was in — and a
    // fleet operator holding a token is not a household.
    deny_unless(state.authority(&headers)?.may_read_everything(&site))?;
    failed(crate::export::data_act(&state.store, &site).await).map(axum::Json)
}

/// `?year=2026&month=10` — the calendar period to settle.
#[derive(Debug, serde::Deserialize)]
pub struct Period {
    /// The calendar year. Required.
    pub year: i32,
    /// The calendar month, `1`–`12`. Required for the Abgrenzungsoption, which
    /// settles per month; refused for the Pauschaloption, which settles per
    /// year.
    #[serde(default)]
    pub month: Option<u8>,
    /// Which **version** of the registers to settle from, RFC 3339.
    ///
    /// Omitted, the settlement reads the registers as they stand, which is what
    /// a first settlement of a period wants. Given, it reads them as they stood
    /// at that instant — which is how a Nachweis already handed over is
    /// reproduced after a register has been restated, and how the delta a
    /// correction produces is computed. The registers are versioned precisely so
    /// this question has an answer.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub as_of: Option<time::OffsetDateTime>,
}

async fn get_mispel(
    State(state): State<History>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Query(period): Query<Period>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Operator access covers § 14a evidence under `[A1 7.2]`. The household
    // credential is required for storage settlement and its underlying flows.
    let allowed = state
        .authority(&headers)
        .and_then(|a| deny_unless(a.may_read_everything(&site)));
    if let Err(status) = allowed {
        return status.into_response();
    }
    let Some(declared) = state.declaration(&site) else {
        // Refused rather than defaulted: every option produces a different
        // Nachweis from the same registers.
        return StatusCode::NOT_FOUND.into_response();
    };
    // A receipt must be replayable even when the caller supplied no cutoff
    // or asked for a time in the future.
    let cutoff = match failed(state.store.settlement_cutoff(period.as_of).await) {
        Ok(cutoff) => cutoff,
        Err(status) => return status.into_response(),
    };
    let outcome = crate::export::mispel(
        &state.store,
        &site,
        Some(declared),
        state.state_aid_approved_on,
        period.year,
        period.month,
        Some(cutoff),
    )
    .await;
    // A refusal says why, in the clause's own words: "the period ends before
    // the installation was assigned" is something the caller can act on, and a
    // bare status is not.
    let explained = |status: StatusCode, e: &crate::export::MispelExportError| {
        (
            status,
            axum::Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response()
    };
    match outcome {
        Ok(value) => axum::Json(value).into_response(),
        // The caller asked wrongly — a window the declared option does not
        // settle over, a month that is not one, or a period the Festlegung
        // does not govern for this site. The last is a `400` rather than a
        // `404` on purpose: the site exists and its option is declared, and what
        // is wrong is the *period* the caller named.
        Err(
            e @ (crate::export::MispelExportError::WrongWindow(_)
            | crate::export::MispelExportError::NotACalendarMonth { .. }
            | crate::export::MispelExportError::NotGoverned { .. }),
        ) => explained(StatusCode::BAD_REQUEST, &e),
        Err(crate::export::MispelExportError::Undeclared { .. }) => {
            StatusCode::NOT_FOUND.into_response()
        }
        // The request was well formed and the **registers** cannot support a
        // settlement — a negative quantity, case A4 without its storage meter.
        // That is a fact about the record rather than about the question,
        // which is what `422` is for.
        Err(e @ crate::export::MispelExportError::Arithmetic(_)) => {
            tracing::warn!(error = %e, "a MiSpeL settlement was refused");
            explained(StatusCode::UNPROCESSABLE_ENTITY, &e)
        }
        Err(crate::export::MispelExportError::Store(e)) => {
            tracing::error!(error = %e, "a query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `403` where a credential is real and does not reach this site.
///
/// Separate from the `401` of [`History::authority`], because "you are nobody"
/// and "you are somebody else" are different facts and an operator debugging a
/// rollout needs to tell them apart.
fn deny_unless(allowed: bool) -> Result<(), StatusCode> {
    if allowed {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
