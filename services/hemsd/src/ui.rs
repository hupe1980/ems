//! The household page, compiled into the binary.
//!
//! Four files and no build step: `cargo build` is the only build, so the page
//! cannot drift from the box that serves it, and a box with no WAN still has
//! its page. The files carry no household data — everything the page shows
//! comes from `GET /v1/home`, behind the same credential every other route is —
//! so they are served **outside** the access layer, the way `/livez` is.

use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use axum::routing::get;

/// The page.
pub const INDEX_HTML: &str = include_str!("../ui/index.html");
/// Its script. Public so a test can hold its sentences to the keys the box sends.
pub const APP_JS: &str = include_str!("../ui/app.js");
/// Its stylesheet.
pub const APP_CSS: &str = include_str!("../ui/app.css");
/// The mark, also the home-screen icon.
pub const ICON_SVG: &str = include_str!("../ui/icon.svg");
/// What a phone needs to put the page on its home screen.
pub const MANIFEST: &str = include_str!("../ui/manifest.webmanifest");

/// A response with a fixed type and the page's security headers.
fn file(body: &'static str, content_type: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // Everything is served by the box itself; nothing may load from
            // anywhere else, and the page may not be framed.
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'self'; img-src 'self' data: blob:; style-src 'self'; \
                     style-src-attr 'unsafe-inline'; script-src 'self'; connect-src 'self'; \
                     frame-ancestors 'none'; base-uri 'none'",
                ),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
}

/// The page's routes: `/` and its four files.
pub fn router<S: Clone + Send + Sync + 'static>() -> axum::Router<S> {
    axum::Router::new()
        .route(
            "/",
            get(|| async { file(INDEX_HTML, "text/html; charset=utf-8") }),
        )
        .route(
            "/app.js",
            get(|| async { file(APP_JS, "text/javascript; charset=utf-8") }),
        )
        .route(
            "/app.css",
            get(|| async { file(APP_CSS, "text/css; charset=utf-8") }),
        )
        .route(
            "/icon.svg",
            get(|| async { file(ICON_SVG, "image/svg+xml") }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async { file(MANIFEST, "application/manifest+json") }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_loads_nothing_from_anywhere_else() {
        for (name, text) in [
            ("index.html", INDEX_HTML),
            ("app.js", APP_JS),
            ("app.css", APP_CSS),
        ] {
            assert!(
                !text.contains("http://") && !text.contains("https://"),
                "{name} names an external address; the page must work with no WAN"
            );
        }
    }

    #[test]
    fn the_page_never_keeps_the_credential_where_a_script_could_read_it() {
        // The token is exchanged once for an HttpOnly cookie. A page that put it
        // in storage would hand it to any script that ever ran on the origin.
        assert!(!APP_JS.contains("localStorage.setItem(\"hems.token"));
        assert!(!APP_JS.contains("sessionStorage"));
        assert!(APP_JS.contains("credentials: \"same-origin\""));
    }
}
