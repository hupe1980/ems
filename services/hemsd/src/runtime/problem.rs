//! One shape for every refusal: RFC 9457 problem documents.
//!
//! The API is published as a contract (`openapi.json`), so the way it says
//! *no* is part of the contract too. Before this module it said it four ways —
//! a sentence as `text/plain`, a bare status, axum's own rejection text, and
//! the sign-in layer's empty 401 — and a client had to know which route spoke
//! which.
//!
//! The fix is one layer rather than sixty edits: every response on `/v1/*`
//! with a status of 400 or above that is not already a problem document
//! becomes one. The handler's sentence, where it wrote one, is the `detail`;
//! the status decides the `type`, a stable anchor on the API page a client can
//! branch on without parsing English. It also catches what no handler writes —
//! an extractor refusing malformed JSON, the access layer's 401 — which is why
//! it is a layer and not a helper.

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The media type of a problem document, RFC 9457 § 3.
pub const MEDIA_TYPE: &str = "application/problem+json";

/// Where each `type` is documented, as an anchor on the API page.
pub const TYPES: &str = "https://hupe1980.github.io/ems/docs/api/#";

/// The largest refusal body read back into a problem. A refusal is a sentence;
/// anything longer is not one, and is replaced rather than read whole.
const MOST: usize = 4 * 1024;

/// An RFC 9457 problem document.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Problem {
    /// A stable URI naming the kind of refusal — an anchor on the API page.
    /// A client branches on this.
    #[serde(rename = "type")]
    pub kind: String,
    /// The kind, in a few words.
    pub title: String,
    /// The HTTP status, repeated so the document stands on its own.
    pub status: u16,
    /// What happened in this case, as a sentence a person can act on.
    pub detail: String,
}

impl Problem {
    /// A problem of the kind `status` names, saying `detail`.
    #[must_use]
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        let (slug, title) = kind_of(status);
        let detail = detail.into();
        Self {
            kind: format!("{TYPES}{slug}"),
            title: title.into(),
            status: status.as_u16(),
            detail: if detail.trim().is_empty() {
                title.into()
            } else {
                detail.trim().to_owned()
            },
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, axum::Json(&self)).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(MEDIA_TYPE));
        response
    }
}

/// The anchor and the title for each status this API refuses with.
///
/// One `type` per status rather than per route: the routes' sentences differ,
/// but what a client can *do* about a 409 does not. A status not listed here is
/// still a problem, under its own reason phrase.
fn kind_of(status: StatusCode) -> (&'static str, &'static str) {
    match status.as_u16() {
        400 => ("bad-request", "The request is not one this box can read"),
        401 => ("unauthorized", "No credential, or not one this box issued"),
        403 => ("forbidden", "This credential may not do that"),
        404 => ("not-found", "There is no such thing on this box"),
        405 => ("method-not-allowed", "That method is not offered here"),
        409 => ("conflict", "The box is not in a state to do that"),
        413 => ("too-large", "The answer would be too large for that"),
        415 => ("unsupported-media-type", "That body is not JSON"),
        422 => (
            "unprocessable",
            "Readable, and not something this box can do",
        ),
        429 => ("too-many", "Too many at once; try again shortly"),
        502 => (
            "bad-gateway",
            "A device or service the box asked did not answer",
        ),
        503 => ("unavailable", "The box cannot answer that yet"),
        500..=599 => ("server-error", "The box failed to do that"),
        _ => ("refused", "Refused"),
    }
}

/// The layer: every refusal on `/v1/*` leaves as a problem document.
///
/// Paths outside `/v1/` are left alone — the page's own files, the S2 socket's
/// upgrade, the health probes — because none of them is the JSON API.
pub async fn as_problem(request: Request, next: Next) -> Response {
    let ours = request.uri().path().starts_with("/v1/");
    let response = next.run(request).await;
    let status = response.status();
    if !ours || status.as_u16() < 400 || is_problem(&response) {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let detail = match axum::body::to_bytes(body, MOST).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    };
    let problem = Problem::new(status, detail);
    let mut out = problem.into_response();
    // Keep what the handler meant beyond the body — `WWW-Authenticate` on a 401,
    // `Retry-After` on a 429 — and replace only what describes the body.
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::CONTENT_TYPE);
    for (name, value) in &parts.headers {
        out.headers_mut()
            .entry(name)
            .or_insert_with(|| value.clone());
    }
    out
}

fn is_problem(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with(MEDIA_TYPE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_names_its_kind_by_an_anchor_and_keeps_the_sentence() {
        let p = Problem::new(StatusCode::NOT_FOUND, "  no such device  ");
        assert_eq!(p.kind, format!("{TYPES}not-found"));
        assert_eq!(p.status, 404);
        assert_eq!(p.detail, "no such device");
    }

    #[test]
    fn a_refusal_with_nothing_to_say_says_its_title() {
        let p = Problem::new(StatusCode::CONFLICT, "");
        assert_eq!(p.detail, p.title);
    }

    #[test]
    fn every_status_this_api_refuses_with_has_its_own_anchor() {
        let statuses = [
            400, 401, 403, 404, 405, 409, 413, 415, 422, 429, 500, 502, 503,
        ];
        let slugs: std::collections::BTreeSet<_> = statuses
            .iter()
            .map(|s| kind_of(StatusCode::from_u16(*s).unwrap()).0)
            .collect();
        assert_eq!(slugs.len(), statuses.len(), "two statuses share an anchor");
        assert!(!slugs.contains("refused"));

        // A `type` is a link, so it must lead somewhere.
        let page = include_str!("../../../../site/content/docs/api.md");
        for slug in slugs {
            assert!(
                page.contains(&format!("id=\"{slug}\"")),
                "the API page has no anchor for #{slug}"
            );
        }
    }
}
