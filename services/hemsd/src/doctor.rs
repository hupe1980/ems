//! Read-only checks for an installer standing at the household gateway.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

/// A check returned to both Setup and the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeviceCheck {
    /// Configured asset identifier.
    pub asset: String,
    /// What the live driver registry observed.
    pub state: DeviceState,
}

/// One device diagnosis. A missing first observation stays unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    /// The registry has run and the configured driver is responding.
    Responding,
    /// Startup has not produced a control tick yet.
    Waiting,
    /// The driver is silent.
    Silent,
    /// The device refused a command.
    Disobedient,
}

impl DeviceState {
    fn line(self) -> &'static str {
        match self {
            Self::Responding => "responding",
            Self::Waiting => "awaiting the first control tick",
            Self::Silent => "silent · check the device connection and power supply",
            Self::Disobedient => {
                "command refused · compare the request with the reading at the device"
            }
        }
    }
}

/// Live diagnostics for the configured assets, including the connection meter.
pub fn devices(
    commissioning: &crate::runtime::handover::Commissioning,
    status: &crate::runtime::control::Status,
) -> Vec<DeviceCheck> {
    use std::collections::BTreeSet;
    let assets: BTreeSet<_> = commissioning
        .modbus
        .iter()
        .chain(&commissioning.eebus)
        .map(|(asset, _)| asset)
        .chain(commissioning.serial.iter().map(|(id, _, _)| id))
        .chain(&commissioning.control_box)
        .collect();
    assets
        .into_iter()
        .map(|asset| {
            let state = if status.at.is_none() {
                DeviceState::Waiting
            } else if status.silent.contains(asset) {
                DeviceState::Silent
            } else if status.disobedient.contains_key(asset) {
                DeviceState::Disobedient
            } else {
                DeviceState::Responding
            };
            DeviceCheck {
                asset: asset.to_string(),
                state,
            }
        })
        .collect()
}

/// Read-only status of stored household quarter-hour summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryState {
    /// The gateway has no configured time-series store.
    Disabled,
    /// No stored ranges await repair, including when the store is new.
    Current,
    /// At least one materialised range needs recomputing.
    RepairPending,
    /// Another history request holds the reader.
    Busy,
    /// The gateway could not inspect the store.
    Unavailable,
}

impl HistoryState {
    fn attention(self) -> Option<&'static str> {
        match self {
            Self::Disabled | Self::Current => None,
            Self::RepairPending => {
                Some("stored quarters await recalculation; check again after automatic maintenance")
            }
            Self::Busy => Some("history reader busy; retry after the current request"),
            Self::Unavailable => Some("history status unavailable; check the gateway log"),
        }
    }
}

/// Installer view of one read-only meter. Values are present only while fresh.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct SerialReading {
    /// Asset assigned on this installation.
    pub asset: String,
    /// Installer-provisioned bus name.
    pub bus: String,
    /// Modbus unit address.
    pub unit: u8,
    /// Manufacturer, model and serial number reported by the attached meter.
    pub identity: Option<String>,
    /// Whether the current session supplied a usable measurement.
    pub fresh: bool,
    /// Receipt time of the most recent accepted measurement, in Unix seconds.
    pub last_read_at: Option<i64>,
    /// Signed connection power. Import is positive.
    pub power_w: Option<f64>,
    /// Cumulative energy imported, in Wh.
    pub import_wh: Option<f64>,
    /// Cumulative energy exported, in Wh.
    pub export_wh: Option<f64>,
}

/// Authenticated installer diagnostics.
#[derive(Serialize, schemars::JsonSchema)]
pub struct Diagnostics {
    /// Current wired heat-meter status, when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heat_meter: Option<crate::runtime::heat_meter::Reading>,
    /// Commissioned serial meters in configuration order.
    pub serial_meters: Vec<SerialReading>,
    /// Existing identity, heartbeat and device checks.
    #[serde(flatten)]
    pub trust: crate::runtime::handover::TrustCheck,
    /// State of quarter-hour summary maintenance.
    pub history: HistoryState,
}

#[derive(Deserialize)]
struct Report {
    heat_meter: Option<crate::runtime::heat_meter::Reading>,
    serial_meters: Vec<SerialReading>,
    history: HistoryState,
    heartbeat_age_s: Option<u64>,
    devices: Vec<DeviceCheck>,
    control_box: ControlBox,
    tick_at: Option<String>,
    identity: String,
}

#[derive(Deserialize)]
struct ControlBox {
    state: String,
}

/// Validate the configuration and inspect the gateway on its configured local socket.
///
/// # Errors
/// Returns an error when configuration, authentication, transport or a live check fails.
pub async fn run(config: &Path, certificate: Option<&Path>) -> anyhow::Result<()> {
    ensure!(config.is_file(), "household configuration file is missing");
    let settings: crate::Settings = ems_service::load(Some(config), "EMS_HEMSD")?;
    crate::runtime::assemble(&settings, None, time::OffsetDateTime::now_utc())?;
    let listen = settings.service.listen;
    let tls = settings.api.tls_on(listen).map_err(anyhow::Error::msg)?;
    println!("configuration  valid");
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let address = SocketAddr::new(ip, listen.port());
    ensure!(
        tls || ip.is_loopback(),
        "use the TLS listener or a loopback listener for doctor"
    );
    let mut client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10));
    if let Some(path) = certificate {
        ensure!(tls, "--ca-cert requires HTTPS");
        let pem = std::fs::read(path).context("read gateway certificate")?;
        client = client.tls_certs_only([reqwest::Certificate::from_pem(&pem)?]);
    }
    let origin = if tls {
        client = client.resolve("hems.local", address);
        format!("https://hems.local:{}", address.port())
    } else {
        format!("http://{address}")
    };
    let token = match std::env::var("HEMS_TOKEN") {
        Ok(token) => token,
        Err(_) => settings
            .api
            .token
            .as_ref()
            .context("set HEMS_TOKEN to the household credential")?
            .resolve_from_process()?,
    };
    let report: Report = client
        .build()?
        .get(format!("{origin}/v1/trust"))
        .bearer_auth(token)
        .send()
        .await
        .context(
            "contact the running gateway; for its self-signed HTTPS certificate use --ca-cert",
        )?
        .error_for_status()
        .context("read household diagnostics with the household credential")?
        .json()
        .await?;
    println!(
        "transport  {}",
        if tls {
            "certificate verified"
        } else {
            "local loopback HTTP"
        }
    );
    report.check(settings.control.tick_period_s)
}

impl Report {
    fn check(self, tick_period_s: u64) -> anyhow::Result<()> {
        let mut good = self.history.attention().is_none();
        if let Some(message) = self.history.attention() {
            println!("history  {message}");
        }
        if let Some(meter) = self.heat_meter {
            println!(
                "heat meter  {} {} · address {} · {}",
                meter.manufacturer,
                meter.meter_id,
                meter.address,
                meter.delivered_heat_wh.map_or_else(
                    || meter.state.message().to_owned(),
                    |wh| format!("{wh} Wh delivered")
                )
            );
            if let Some(received) = meter
                .last_read_at
                .and_then(|at| time::OffsetDateTime::from_unix_timestamp(at).ok())
                .and_then(|at| {
                    at.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                })
            {
                println!("last heat reading  {received}");
            }
            good &= meter.state == crate::runtime::heat_meter::State::Live;
        }
        for device in self.devices {
            if let Some(meter) = self.serial_meters.iter().find(|m| m.asset == device.asset) {
                println!(
                    "{}  {} / unit {}  {}",
                    device.asset,
                    meter.bus,
                    meter.unit,
                    device.state.line()
                );
                good &= meter.fresh;
            } else {
                println!("{}  {}", device.asset, device.state.line());
            }
            good &= device.state == DeviceState::Responding;
        }
        let pairing = match self.control_box.state.as_str() {
            "unpaired" => " · approve the expected Steuerbox in Setup",
            "silent" | "failsafe" => " · check the Steuerbox connection and heartbeat",
            _ => "",
        };
        println!("control box  {}{pairing}", self.control_box.state);
        if let Some(seconds) = self.heartbeat_age_s {
            println!("last control heartbeat  {seconds} s ago");
        }
        good &= matches!(self.control_box.state.as_str(), "none" | "heard");
        let identity_fix = if self.identity == "ephemeral" {
            " · configure a persistent gateway store"
        } else {
            ""
        };
        println!("identity  {}{identity_fix}", self.identity);
        good &= self.identity != "ephemeral";
        let age = self
            .tick_at
            .and_then(|stamp| {
                time::OffsetDateTime::parse(&stamp, &time::format_description::well_known::Rfc3339)
                    .ok()
            })
            .map(|at| (time::OffsetDateTime::now_utc() - at).whole_seconds());
        let recent = age.is_some_and(|s| {
            s >= -1 && s.unsigned_abs() <= tick_period_s.saturating_mul(3).max(10)
        });
        println!(
            "control clock  {}",
            if recent {
                "recent tick"
            } else {
                "check the gateway clock and control loop"
            }
        );
        ensure!(good && recent, "gateway checks need attention");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{control::Status, handover::Commissioning};
    use ems_core::prelude::AssetId;

    #[test]
    fn history_attention_affects_the_doctor_exit_status() {
        for (state, ok) in [
            ("disabled", true),
            ("current", true),
            ("repair_pending", false),
            ("busy", false),
            ("unavailable", false),
        ] {
            let report: Report = serde_json::from_value(serde_json::json!({
                "history":state, "heartbeat_age_s":null, "devices":[], "serial_meters":[], "identity":"kept",
                "control_box":{"state":"none"}, "tick_at":time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap()
            })).unwrap();
            assert_eq!(report.check(1).is_ok(), ok, "{state}");
        }
    }

    #[test]
    fn the_connection_meter_is_visible_before_during_and_after_silence() {
        let meter = AssetId::new("connection-meter").unwrap();
        let config = Commissioning {
            modbus: vec![(meter.clone(), "127.0.0.1:1502".into())],
            ..Commissioning::default()
        };
        let mut status = Status::default();
        assert_eq!(devices(&config, &status)[0].state, DeviceState::Waiting);
        status.at = Some(time::OffsetDateTime::UNIX_EPOCH);
        status.silent.push(meter);
        let report = devices(&config, &status);
        assert_eq!(report[0].asset, "connection-meter");
        assert_eq!(report[0].state, DeviceState::Silent);
        status.silent.clear();
        assert_eq!(devices(&config, &status)[0].state, DeviceState::Responding);
    }
}
