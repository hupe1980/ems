//! Household gateway for daily energy history and charging receipts.
use std::path::PathBuf;

use anyhow::{Context, ensure};
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use clap::Parser;
use ems_service::{Health, Server};
use serde::Deserialize;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

#[derive(Parser)]
struct Cli {
    #[arg(long, env = "EMS_PORTALD_CONFIG")]
    config: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    service: ems_service::Settings,
    collector_origin: String,
    assets_dir: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            service: ems_service::Settings {
                listen: ([127, 0, 0, 1], 7680).into(),
                ..Default::default()
            },
            collector_origin: "http://127.0.0.1:7780".into(),
            assets_dir: "web/portal/dist".into(),
        }
    }
}

impl AsMut<ems_service::Settings> for Settings {
    fn as_mut(&mut self) -> &mut ems_service::Settings {
        &mut self.service
    }
}

#[derive(Clone)]
struct Collector {
    client: reqwest::Client,
    receipt: reqwest::Url,
}

impl Collector {
    fn new(config: &Settings) -> anyhow::Result<Self> {
        let origin = reqwest::Url::parse(&config.collector_origin)?;
        ensure!(
            origin.has_host()
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none(),
            "collector_origin must be an origin"
        );
        ems_service::http::confidential(origin.as_str())?;
        let client =
            ems_service::http::client_builder(ems_service::identity!(), &config.service.http)?
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
        Ok(Self {
            client,
            receipt: origin.join("/v1/me/departure")?,
        })
    }

    async fn read(&self, headers: &HeaderMap, path: &str) -> Result<Response, StatusCode> {
        let token = ems_service::auth::bearer(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
        )
        .ok_or(StatusCode::UNAUTHORIZED)?;
        let mut upstream = self
            .client
            .get(
                self.receipt
                    .join(path)
                    .map_err(|_| StatusCode::BAD_GATEWAY)?,
            )
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        match upstream.status() {
            StatusCode::OK => {}
            status
            @ (StatusCode::NO_CONTENT | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
                return Ok(status.into_response());
            }
            _ => return Err(StatusCode::BAD_GATEWAY),
        }
        let mut body = Vec::new();
        while let Some(chunk) = upstream
            .chunk()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?
        {
            if chunk.len() > 16384 - body.len() {
                return Err(StatusCode::BAD_GATEWAY);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<serde_json::Value>(&body).map_err(|_| StatusCode::BAD_GATEWAY)?;
        Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
    }
}

async fn receipt(State(collector): State<Collector>, headers: HeaderMap) -> Response {
    (
        [(header::CACHE_CONTROL, "private, no-store")],
        collector.read(&headers, "departure").await,
    )
        .into_response()
}

async fn history(State(collector): State<Collector>, headers: HeaderMap) -> Response {
    (
        [(header::CACHE_CONTROL, "private, no-store")],
        collector.read(&headers, "history").await,
    )
        .into_response()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Settings = ems_service::load(Cli::parse().config.as_deref(), "EMS_PORTALD")?;
    ems_service::init_tracing(
        ems_service::identity!(),
        &config.service.log_filter,
        config.service.log_json,
    );
    let index = config.assets_dir.join("index.html");
    std::fs::metadata(&index)
        .context("portal build missing: run just portal-build or configure assets_dir")?;
    let policy = HeaderValue::from_static(
        "default-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    );
    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/portal") }))
        .route("/portal/", get(|| async { Redirect::permanent("/portal") }))
        .route_service("/portal", ServeFile::new(index))
        .nest_service("/assets", ServeDir::new(config.assets_dir.join("assets")))
        .route("/v1/me/history", get(history))
        .route("/v1/me/departure", get(receipt))
        .with_state(Collector::new(&config)?)
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            policy,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
    Server::new(ems_service::identity!(), config.service, Health::new(), app)
        .run()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
