//! Commission a new household from verified SunSpec endpoints and installation facts.
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use super::access::LocalAccess;
use super::installation::{Device, DeviceRole, Installer, MeterEndpoint, Receipt};

#[derive(Clone)]
struct Setup {
    installer: Arc<Installer>,
    ports: ProvisionedPorts,
    probing: Arc<tokio::sync::Mutex<()>>,
    stop: ems_service::shutdown::ShutdownTrigger,
    store_path: String,
}

/// Preconfigured ports retained when commissioning writes the first household configuration.
#[derive(Clone, Serialize)]
struct ProvisionedPorts {
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    serial_buses: std::collections::BTreeMap<String, crate::config::SerialBus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    heat_meter: Option<crate::config::HeatMeterSettings>,
}

/// One observed asset, identified by its endpoint and installation sheet.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedAsset {
    /// Private Modbus endpoint.
    pub address: SocketAddr,
    /// Unit at the endpoint.
    pub unit: u8,
    /// AC nameplate power in kW.
    pub power_kw: f64,
    /// PV array kWp or usable battery capacity in kWh, as labeled in Setup.
    pub size: f64,
    /// Date on the installer's commissioning sheet.
    pub commissioned: String,
}

/// Installer-entered facts for the first three-phase meter.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FirstMeter {
    /// Literal private IPv4 or IPv6 address and one supported Modbus port.
    pub address: Option<SocketAddr>,
    /// Name of an installer-provisioned serial bus, for an RTU meter.
    pub serial_bus: Option<String>,
    /// Modbus unit selected by the installer.
    pub unit: u8,
    /// Household main fuse per phase.
    pub fuse_a: u16,
    /// Number of phases at this connection.
    pub phases: u8,
    /// Contracted net energy rate.
    pub energy_ct: f64,
    /// Net variable network charge.
    pub network_ct: f64,
    /// Total fixed standing charge in euros per year.
    pub standing_eur: f64,
    /// Optional name and telephone number.
    pub support: String,
    /// Optional verified inverter observed in shadow mode.
    pub roof: Option<ObservedAsset>,
    /// Optional verified battery observed in shadow mode.
    pub storage: Option<ObservedAsset>,
    /// Independently commissioned controller retaining authority over observed assets.
    pub external_limit_controller: Option<String>,
}

/// A validated initial site with verified device identities.
#[derive(Serialize, schemars::JsonSchema)]
pub struct MeterReview {
    /// Exact, validated configuration addition.
    pub toml: String,
    /// Device-reported name shown separately from the candidate.
    pub device: String,
    /// Server-issued id for confirmation.
    pub proposal: String,
}

type Refusal = (StatusCode, &'static str);

async fn preview(
    State(setup): State<Setup>,
    Json(ask): Json<FirstMeter>,
) -> Result<Json<MeterReview>, Refusal> {
    if ask.phases != 3 || !(1..=400).contains(&ask.fuse_a) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Select a three-phase connection and enter its fuse rating (1–400 A).",
        ));
    }
    if ![ask.energy_ct, ask.network_ct]
        .iter()
        .all(|v| (0.0..=1000.0).contains(v))
        || !(0.0..=100_000.0).contains(&ask.standing_eur)
        || ask.support.len() > 160
        || ask.support.chars().any(char::is_control)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Check the contract rates and contact details.",
        ));
    }
    let endpoint = match (&ask.serial_bus, ask.address) {
        (None, Some(address)) => MeterEndpoint::Tcp { address },
        (Some(name), None) if (1..=247).contains(&ask.unit) => MeterEndpoint::Serial {
            bus: setup
                .ports
                .serial_buses
                .get(name)
                .ok_or((StatusCode::BAD_REQUEST, "Choose a configured serial bus."))?
                .clone(),
        },
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Choose one endpoint and a valid meter unit.",
            ));
        }
    };
    let _probe = setup
        .probing
        .try_lock()
        .map_err(|_| (StatusCode::CONFLICT, "Device review already in progress."))?;
    let (devices, labels) = ask.probe_devices(endpoint).await?;
    let mut block = ask.block(
        &setup.store_path,
        devices.iter().any(|d| d.role == DeviceRole::Pv),
    );
    block.push_str(&toml::to_string(&setup.ports).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid converter configuration.",
        )
    })?);
    let candidate: crate::Settings = toml::from_str(&block).map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "This site configuration could not be read.",
        )
    })?;
    super::assemble(&candidate, None, time::OffsetDateTime::now_utc()).map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "The gateway refused this configuration.",
        )
    })?;
    let id = setup
        .installer
        .offer_site(&block, devices)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration unavailable.",
            )
        })?
        .ok_or((StatusCode::CONFLICT, "Configuration changed. Reload Setup."))?;
    Ok(Json(MeterReview {
        toml: block,
        device: labels.join(" / "),
        proposal: id,
    }))
}

impl FirstMeter {
    async fn probe_devices(
        &self,
        endpoint: MeterEndpoint,
    ) -> Result<(Vec<Device>, Vec<String>), Refusal> {
        let identity = endpoint.inspect(self.unit)
        .await
        .filter(|id| id.connection_meter)
        .ok_or((
            StatusCode::UNPROCESSABLE_ENTITY,
            "No supported SunSpec connection meter (model 203) answered at this address and unit.",
        ))?;
        let mut devices = vec![Device::from_identity(
            DeviceRole::ConnectionMeter,
            endpoint,
            self.unit,
            &identity,
        )];
        let mut labels = vec![format!(
            "{} {} · {}",
            identity.manufacturer, identity.model, identity.serial
        )];
        let mut endpoints = self
            .address
            .map(|address| (address, self.unit))
            .into_iter()
            .collect::<Vec<_>>();
        if self.roof.is_some() || self.storage.is_some() {
            let controller = self
                .external_limit_controller
                .as_deref()
                .unwrap_or_default()
                .trim();
            if controller.is_empty()
                || controller.len() > 160
                || controller.chars().any(char::is_control)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Name the existing controller that independently enforces this site's limits.",
                ));
            }
        }
        for (role, asset) in [
            (DeviceRole::ObservedPv, self.roof.as_ref()),
            (DeviceRole::ObservedBattery, self.storage.as_ref()),
        ] {
            let Some(asset) = asset else { continue };
            if endpoints.contains(&(asset.address, asset.unit))
                || ![asset.size, asset.power_kw]
                    .into_iter()
                    .all(|v| v > 0.0 && v <= 1000.0)
                || asset.commissioned.len() != 10
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Check the distinct device units, commissioning dates and nameplate values.",
                ));
            }
            endpoints.push((asset.address, asset.unit));
            let refusal = match role {
                DeviceRole::ObservedPv => {
                    "No supported inverter measurement model (101–103) answered at the roof address and unit."
                }
                _ => {
                    "No supported battery measurement model (802) answered at the storage address and unit."
                }
            };
            let id = super::handover::inspect(asset.address, asset.unit)
                .await
                .filter(|id| role.supports(id))
                .ok_or((StatusCode::UNPROCESSABLE_ENTITY, refusal))?;
            let role = if role == DeviceRole::ObservedPv && id.supports_pv_driver() {
                DeviceRole::Pv
            } else {
                role
            };
            devices.push(Device::from_identity(
                role,
                MeterEndpoint::Tcp {
                    address: asset.address,
                },
                asset.unit,
                &id,
            ));
            labels.push(format!("{} {} · {}", id.manufacturer, id.model, id.serial));
        }
        Ok((devices, labels))
    }

    fn block(&self, store_path: &str, verified_curtailment: bool) -> String {
        let contact = toml::Value::String(self.support.trim().to_owned());
        let store = toml::Value::String(store_path.to_owned());
        let mut block = format!("store_path = {store}\n");
        if !self.support.trim().is_empty() {
            writeln!(&mut block, "support_contact = {contact}").expect("writing a String");
        }
        if let Some(controller) = &self.external_limit_controller
            && (self.roof.is_some() || self.storage.is_some())
        {
            writeln!(
                &mut block,
                "mode = \"shadow\"\nexternal_limit_controller = {}",
                toml::Value::String(controller.trim().to_owned())
            )
            .expect("writing a String");
        }
        let (pv_kwp, pv_ac_kw) = self
            .roof
            .as_ref()
            .map_or((0.0, 0.0), |r| (r.size, r.power_kw));
        let (battery_kwh, battery_kw) = self
            .storage
            .as_ref()
            .map_or((0.0, 0.0), |r| (r.size, r.power_kw));
        let endpoint = match &self.serial_bus {
            Some(bus) => format!(
                "kind = \"serial-sunspec\"\nbus = {}",
                toml::Value::String(bus.clone())
            ),
            None => format!(
                "kind = \"sunspec\"\naddress = \"{}\"\nlistens_only = true",
                self.address.expect("TCP endpoint validated")
            ),
        };
        write!(
            &mut block,
            r#"
[api]
tls = true

[site]
fuse_a = {}
pv_kwp = {pv_kwp:?}
pv_ac_kw = {pv_ac_kw:?}
battery_kwh = {battery_kwh:?}
battery_kw = {battery_kw:?}
evse_max_a = 0.0
heat_pump_kw = 0.0
dhw_litres = 0.0
dishwasher_kw_steps = []
[site.para9]
[site.declared]

[tariff]
fixed_ct_per_kwh = {:?}
network_ct_per_kwh = {:?}
standing_charge_eur_per_year = {:?}
modul = "none"

[[drivers]]
{endpoint}
asset = "netzanschluss-zaehler"
unit = {}
"#,
            self.fuse_a, self.energy_ct, self.network_ct, self.standing_eur, self.unit
        )
        .expect("writing a String");
        for (role, asset) in [
            (DeviceRole::ObservedPv, self.roof.as_ref()),
            (DeviceRole::ObservedBattery, self.storage.as_ref()),
        ] {
            if let Some(a) = asset {
                let id = role.asset();
                let date = toml::Value::String(a.commissioned.clone());
                let capabilities = if role == DeviceRole::ObservedPv && verified_curtailment {
                    format!("rating_kw = {:?}", a.power_kw)
                } else {
                    "listens_only = true".into()
                };
                writeln!(&mut block,"\n[site.declared.{id}]\ncommissioned = {date}\n\n[[drivers]]\nkind = \"sunspec\"\nasset = \"{id}\"\naddress = \"{}\"\nunit = {}\n{capabilities}",a.address,a.unit).expect("writing a String");
            }
        }
        block
    }
}

/// Serve first-run commissioning under the permanent gateway credential and TLS identity.
///
/// # Errors
/// Returns an error if the persistent identity or listener cannot be opened.
pub async fn run(
    path: &Path,
    settings: &crate::Settings,
    installer: Arc<Installer>,
) -> anyhow::Result<()> {
    settings.validate_serial().map_err(anyhow::Error::msg)?;
    let store_path = path.canonicalize()?.with_added_extension("redb");
    let store = crate::store::Store::open(&store_path)?;
    let (access, issued) = LocalAccess::resolve(&settings.api, "first-run", Some(&store))?;
    if let Some(line) = issued.announcement() {
        println!("{line}");
    }
    let (signal, stop) = ems_service::Shutdown::channel();
    tokio::spawn(ems_service::shutdown::on_signal(stop.clone()));
    let setup = Setup {
        ports: ProvisionedPorts {
            serial_buses: settings.serial_buses.clone(),
            heat_meter: settings.heat_meter.clone(),
        },
        probing: Arc::default(),
        installer,
        stop,
        store_path: store_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("configuration path requires UTF-8"))?
            .to_owned(),
    };
    let router = Router::new()
        .route(
            "/v1/commissioning/serial-buses",
            get(|State(setup): State<Setup>| async move {
                Json(setup.ports.serial_buses.into_keys().collect::<Vec<_>>())
            }),
        )
        .route("/v1/commissioning/first-meter", post(preview))
        .route("/v1/commissioning/change", get(outcome).post(apply))
        .with_state(setup)
        .layer(axum::middleware::from_fn_with_state(
            access.clone(),
            super::access::require_token,
        ))
        .merge(super::access::session_router(access, true))
        .route(
            "/",
            get(|| async {
                ems_service::edge::file(
                    include_str!("../../ui/first-run.html"),
                    "text/html; charset=utf-8",
                )
            }),
        )
        .route(
            "/first-run.js",
            get(|| async {
                ems_service::edge::file(
                    include_str!("../../ui/first-run.js"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route(
            "/app.css",
            get(|| async {
                ems_service::edge::file(crate::ui::APP_CSS, "text/css; charset=utf-8")
            }),
        )
        .layer(axum::middleware::from_fn(super::problem::as_problem));
    let health = ems_service::Health::new();
    health.bad("commissioning", "waiting for the first confirmed meter");
    let server = ems_service::Server::new(
        ems_service::identity!(),
        settings.service.clone(),
        health,
        router,
    );
    let server = server.with_tls(super::tls::server_config(Some(&store))?.0);
    println!(
        "Open https://{}/ to commission the connection meter.",
        settings.service.listen
    );
    server.run_until(signal).await?;
    Ok(())
}

async fn outcome(State(setup): State<Setup>) -> Result<Json<Option<Receipt>>, StatusCode> {
    setup
        .installer
        .receipt()
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn apply(
    State(setup): State<Setup>,
    who: Option<axum::Extension<ems_service::auth::Authority>>,
    Json(request): Json<super::api::ApplyInstallation>,
) -> Result<Json<Receipt>, (StatusCode, String)> {
    super::api::commit_installation(setup.installer, setup.stop, who.as_deref(), request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_bootstrap_validates_without_inheriting_reference_devices_or_declarations() {
        let input = FirstMeter {
            address: Some("127.0.0.1:1502".parse().unwrap()),
            serial_bus: None,
            unit: 2,
            fuse_a: 35,
            phases: 3,
            energy_ct: 20.0,
            network_ct: 7.2,
            standing_eur: 99.0,
            roof: None,
            storage: None,
            external_limit_controller: None,
            support: "Installer \"A\"".into(),
        };
        let config: crate::Settings = toml::from_str(&input.block("home.redb", false)).unwrap();
        let running =
            super::super::assemble(&config, None, time::OffsetDateTime::now_utc()).unwrap();
        assert!(running.household.pv.is_none());
        assert!(running.household.battery.is_none());
        assert!(running.household.evse.is_none());
        assert!(config.site.declared.is_empty());
        assert_eq!(config.drivers.len(), 1);
        assert_eq!(config.support_contact.as_deref(), Some("Installer \"A\""));
    }
}
