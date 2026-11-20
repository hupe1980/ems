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

/// The service worker: shows a notification the box pushed, in browsers that
/// do not show one on their own.
pub const SW_JS: &str = include_str!("../ui/sw.js");

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
        // At the root, so its scope is the whole page: a worker controls only
        // what is at or beneath its own path. `default-src 'self'` is what lets
        // the page register it — `worker-src` falls back to `script-src`.
        .route(
            "/sw.js",
            get(|| async { file(SW_JS, "text/javascript; charset=utf-8") }),
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
            ("sw.js", SW_JS),
        ] {
            assert!(
                !text.contains("http://") && !text.contains("https://"),
                "{name} names an external address; the page must work with no WAN"
            );
        }
    }

    #[test]
    fn the_service_worker_only_shows_and_opens() {
        // It is handed the household's notifications, so it must do nothing
        // else: no fetch handler to sit between the page and the box, no cache
        // to keep a document after sign-out, no request of its own.
        for forbidden in [
            "fetch",
            "caches",
            "importScripts",
            "XMLHttpRequest",
            "indexedDB",
        ] {
            assert!(!SW_JS.contains(forbidden), "sw.js uses `{forbidden}`");
        }
        for needed in [
            "\"push\"",
            "\"notificationclick\"",
            "web_push",
            "showNotification",
        ] {
            assert!(SW_JS.contains(needed), "sw.js lost `{needed}`");
        }
    }

    #[test]
    fn the_wall_display_offers_nothing_to_press() {
        // A tablet on a wall is in reach of every visitor: the glance face is one
        // link back to Now and no control, fingerprint or code.
        let start = APP_JS.find("function viewGlance").expect("the glance view");
        let face = &APP_JS[start..start + APP_JS[start..].find("\n}\n").expect("its end")];
        for forbidden in [
            "<button",
            "<input",
            "<select",
            "<form",
            "h.ski",
            "fingerprint",
            "/v1/qr",
        ] {
            assert!(
                !face.contains(forbidden),
                "the glance face contains {forbidden}"
            );
        }
        assert_eq!(face.matches("href=").count(), 1);
        assert!(face.contains("href=\"#now\""));
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
