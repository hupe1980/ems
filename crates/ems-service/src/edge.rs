//! HTTP helpers for local edge user interfaces.
use axum::{
    Router,
    http::{HeaderValue, header},
    response::IntoResponse,
    routing::get,
};
use std::net::SocketAddr;

/// A response with a fixed type and the page's security headers.
pub fn file(body: impl IntoResponse, content_type: &'static str) -> impl IntoResponse {
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

/// Serve a synthetic operator screen and its stylesheet on loopback.
///
/// # Errors
/// Returns an error for an external bind address or a listener failure.
pub async fn serve_demo(router: Router, listen: SocketAddr) -> anyhow::Result<()> {
    anyhow::ensure!(
        listen.ip().is_loopback(),
        "demo requires a loopback address"
    );
    let router = router.route(
        "/app.css",
        get(async || file(include_str!("edge.css"), "text/css; charset=utf-8")),
    );
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!("Open http://{}/", listener.local_addr()?);
    axum::serve(listener, router).await?;
    Ok(())
}
