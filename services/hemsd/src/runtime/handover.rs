//! What an installer carries away, and what the box can find for them.
//!
//! Four small things, all of them commissioning's:
//!
//! - **QR codes** drawn here as SVG, so the household page stays without a
//!   script that could draw one. The SHIP code is the standard's own payload
//!   (`SHIP;SKI:…;ID:…;FPH256:…;ENDSHIP;`), which is what a pairing tool scans.
//! - **The handover sheet** as one document: the box's identities and each
//!   controllable device with what `[BK6-22-300 A1 4.5]` makes of it.
//! - **Discovery**: SHIP nodes announcing themselves, and a SunSpec probe of the
//!   hosts an installer names. Discovery proposes; the configuration decides.
//! - **Trust**: whether the identity the box presents is the one its store will
//!   restore, and whether the control box is still being heard.
//! - **The commissioning record**: the sheet's devices and the trust check as
//!   one statement the box signs, for whoever ordered the installation.
//!
//! # The record is signed with the SHIP key, and why that is sound
//!
//! Whoever ordered the installation already holds this box's SKI — it is what
//! they gave the metering point operator — so a statement signed by the key
//! behind that SKI needs no second identity to be handed over and compared. A
//! second key certified by the first would bind to the same SKI through one
//! more signature and one more thing to keep, and prove nothing more.
//!
//! The cost is one key signing in two protocols. What keeps them apart: every
//! signing input here is `base64url(header).base64url(payload)` and begins with
//! this record's own protected header, `typ: hems-commissioning+jws`; a TLS 1.2
//! key-exchange signature covers 64 octets of random and binary curve
//! parameters, and a TLS 1.3 one begins with 64 spaces and a context string.
//! Neither can be read as the other. The only text in the payload a caller
//! chooses is `reference`, which [`is_reference`] holds to printable ASCII.
//!
//! # The probe is a request to open sockets, so it is narrow
//!
//! An endpoint that connects to whatever address it is handed is a port scanner
//! with the box's position on the network. So: the household's own credential
//! only; literal addresses on a private network only — no names, so nothing
//! resolves; port 502, or 1502 where asked, and no other; at most [`MAX_HOSTS`]
//! an ask; one short
//! timeout for the lot; reads only. And **a configured device is never probed**:
//! many inverters serve exactly one Modbus TCP client, and a second connection
//! from the box would throw its own driver off the first. What the box already
//! knows about a configured device — heard or silent — is the answer.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::time::Duration;

use ems_core::asset::Fallgruppe;
use ems_core::prelude::{Asset, AssetId, Site};
use ems_drv::modbus::frame;
use ems_drv::modbus::probe::{Identity, Probe};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The most hosts one discovery may name.
pub const MAX_HOSTS: usize = 16;

/// The ports a probe may dial: Modbus TCP's own, and the 1502 that Kostal and
/// many SolarEdge installations serve SunSpec on. These two and no other.
pub const MODBUS_PORTS: [u16; 2] = [502, 1502];

/// How long one host has to answer everything a probe asks.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// How long to listen for `_ship._tcp` announcements.
const BROWSE_FOR: Duration = Duration::from_millis(1500);

/// What the configuration says about this box's devices, as far as
/// commissioning needs it. Settled before the first request and never changed.
#[derive(Debug, Clone, Default)]
pub struct Commissioning {
    /// The SHIP ID this box announces itself under.
    pub ship_id: Option<String>,
    /// The asset the network operator's control box speaks for, where one is
    /// configured.
    pub control_box: Option<AssetId>,
    /// Each configured Modbus device: its asset and the `host:port` it is at.
    pub modbus: Vec<(AssetId, String)>,
    /// Configured RTU meter asset, bus name and unit.
    pub serial: Vec<(AssetId, String, u8)>,
    /// Each configured EEBUS device on the household's network: asset and SKI.
    pub eebus: Vec<(AssetId, String)>,
    /// The § 14a module the household is billed under, as configured.
    pub module: Option<String>,
}

impl Commissioning {
    /// What `settings` names.
    #[must_use]
    pub fn of(settings: &crate::config::Settings) -> Self {
        use crate::config::DriverSettings;
        let asset = |name: &str| AssetId::new(name).ok();
        let mut out = Self {
            ship_id: Some(settings.ship.ship_id.clone()),
            module: serde_json::to_value(settings.tariff.modul)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned)),
            ..Self::default()
        };
        for driver in &settings.drivers {
            let Some(id) = asset(driver.asset()) else {
                continue;
            };
            match driver {
                DriverSettings::Sunspec(s) => out.modbus.push((id, s.address.clone())),
                DriverSettings::SerialSunspec(s) => out.serial.push((id, s.bus.clone(), s.unit)),
                DriverSettings::Registers(s) => out.modbus.push((id, s.address.clone())),
                DriverSettings::EebusLpc(_) => out.control_box = Some(id),
                DriverSettings::EebusDhw(s) => out.eebus.push((id, s.ski.clone())),
                DriverSettings::EebusEv(s) => out.eebus.push((id, s.ski.clone())),
                DriverSettings::EebusHeatPump(s) => out.eebus.push((id, s.ski.clone())),
            }
        }
        out
    }
}

// ── QR codes ────────────────────────────────────────────────────────────────

/// `text` as a QR code, SVG.
///
/// # Errors
/// Where the text is longer than a QR code holds.
pub fn qr_svg(text: &str) -> Result<String, qrcode::types::QrError> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M)?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// The SHIP pairing payload for this box: its SKI, its SHIP ID and its
/// certificate's fingerprint.
///
/// `None` where a part is not what SHIP would read back — which no value this
/// box produces is, and a code nothing can scan is worse than none.
#[must_use]
pub fn ship_payload(ski: &str, ship_id: &str, fingerprint: Option<&str>) -> Option<String> {
    let mut qr = eebus::ship::ShipQr::new(ski.parse().ok()?, ship_id.parse().ok()?);
    // SHIP writes a fingerprint in upper case and reads no other.
    qr.certificate_fingerprint = fingerprint.and_then(|f| f.to_ascii_uppercase().parse().ok());
    Some(qr.to_payload())
}

/// The link the sign-in code carries: the page, with the token in the fragment.
///
/// The fragment is never sent to a server, so the token reaches no access log,
/// and the page wipes it from the address bar once it has signed in.
#[must_use]
pub fn sign_in_link(origin: &str, token: &str) -> String {
    format!("{origin}/#t={token}")
}

/// Whether `origin` is `http(s)://host[:port]` and nothing more.
///
/// It ends up inside a QR code somebody will scan, so it may not carry a path,
/// a query, a credential or anything a scanner would read as a second link.
#[must_use]
pub fn is_origin(origin: &str) -> bool {
    let Some(host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    is_host(host)
}

/// Whether `host` is a `Host` header's worth: a name or address, with an
/// optional port.
#[must_use]
pub fn is_host(host: &str) -> bool {
    (1..=255).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

// ── The handover sheet ──────────────────────────────────────────────────────

/// Everything the A4 sheet prints.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Handover {
    /// The household this box is.
    pub site: String,
    /// When the sheet was made, RFC 3339.
    pub generated_at: String,
    /// The box's SKI — what the metering point operator has to be given.
    pub ski: Option<String>,
    /// The SHIP ID it announces itself under.
    pub ship_id: Option<String>,
    /// The SHA-256 fingerprint of its SHIP certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate the page is served with —
    /// what a browser's warning shows on first visit.
    pub https_fingerprint: Option<String>,
    /// How the network operator addresses this household, `[A1 4.4]`: always
    /// `ems` here — a box that manages the devices is what an EMS is.
    pub control: &'static str,
    /// The § 14a module the household is billed under, as configured.
    pub module: Option<String>,
    /// The power a reduction may not go below, watts, `[A1 4.5.2]`.
    pub minimum_w: f64,
    /// Each steuerbare Verbrauchseinrichtung, as the network operator's
    /// registration asks for it.
    pub devices: Vec<Device>,
}

/// One steuerbare Verbrauchseinrichtung.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[schemars(rename = "SheetDevice")]
pub struct Device {
    /// The assets that make it up — one, or a whole Fallgruppe summed.
    pub assets: Vec<String>,
    /// What the household calls them.
    pub labels: Vec<String>,
    /// `ladepunkt`, `waermepumpe`, `raumkuehlung` or `stromspeicher`.
    pub fallgruppe: &'static str,
    /// The letter `[A1 2.4.1]` lists it under: `a`–`d`.
    pub letter: &'static str,
    /// Its Netzanschlussleistung, watts.
    pub power_w: f64,
    /// What it alone would be owed under direct control, watts, `[A1 4.5.1]`.
    pub minimum_direct_w: f64,
    /// The date of technical commissioning, where the configuration states one
    /// for every asset in it; the earliest.
    pub commissioned: Option<String>,
}

/// The sheet for `site` as of `now`.
#[must_use]
pub fn sheet(
    site: &Site,
    now: OffsetDateTime,
    ski: Option<&str>,
    fingerprint: Option<&str>,
    https_fingerprint: Option<&str>,
    commissioning: &Commissioning,
) -> Handover {
    use ems_grid::para14a::{ControlMode, MINDESTLEISTUNG, minimum_power};
    let classified = ems_grid::classify_at(&site.assets, now);
    let asset = |id: &AssetId| site.assets.iter().find(|a| a.id() == id);
    let devices = classified
        .iter()
        .map(|steuve| {
            let members: Vec<&Asset> = steuve.assets.iter().filter_map(asset).collect();
            let (fallgruppe, letter) = name(steuve.fallgruppe);
            Device {
                assets: steuve.assets.iter().map(ToString::to_string).collect(),
                labels: members.iter().map(|a| label(a)).collect(),
                fallgruppe,
                letter,
                power_w: steuve.power.get(),
                minimum_direct_w: steuve.minimum_power_direct().get(),
                commissioned: members
                    .iter()
                    .filter_map(|a| a.meta().commissioned_at)
                    .min()
                    .map(|d| d.to_string()),
            }
        })
        .collect();
    Handover {
        site: site.id.to_string(),
        generated_at: rfc3339(now),
        ski: ski.map(str::to_owned),
        ship_id: commissioning.ship_id.clone(),
        fingerprint: fingerprint.map(str::to_owned),
        https_fingerprint: https_fingerprint.map(str::to_owned),
        control: "ems",
        module: commissioning.module.clone(),
        // The same arithmetic the failsafe default uses (`runtime::driver_for`),
        // so the sheet and the box cannot state two minimums.
        minimum_w: minimum_power(&classified, ControlMode::Ems)
            .max(MINDESTLEISTUNG)
            .get(),
        devices,
    }
}

fn name(fallgruppe: Fallgruppe) -> (&'static str, &'static str) {
    match fallgruppe {
        Fallgruppe::Ladepunkt => ("ladepunkt", "a"),
        Fallgruppe::Waermepumpe => ("waermepumpe", "b"),
        Fallgruppe::Raumkuehlung => ("raumkuehlung", "c"),
        Fallgruppe::Stromspeicher => ("stromspeicher", "d"),
        // `metering` may learn a Fallgruppe before this sheet does; it is then
        // printed as unknown rather than under somebody else's letter.
        _ => ("unbekannt", "?"),
    }
}

fn label(asset: &Asset) -> String {
    let meta = asset.meta();
    if meta.label.is_empty() {
        meta.id.to_string()
    } else {
        meta.label.clone()
    }
}

// ── The commissioning record ────────────────────────────────────────────────

/// The `typ` of a commissioning record's protected header: what separates it
/// from every other thing an ES256 key might sign.
pub const RECORD_TYP: &str = "hems-commissioning+jws";

/// The longest reference a record echoes.
pub const MAX_REFERENCE: usize = 64;

/// Whether `reference` may be echoed: 1–[`MAX_REFERENCE`] printable ASCII
/// characters, no space.
///
/// It is somebody else's identifier — an order number — and is never parsed
/// here, only signed. Printable ASCII keeps it the same bytes in a QR code, a
/// URL and a database column, and keeps a caller from choosing octets of the
/// signing input.
#[must_use]
pub fn is_reference(reference: &str) -> bool {
    (1..=MAX_REFERENCE).contains(&reference.len())
        && reference.bytes().all(|b| b.is_ascii_graphic())
}

/// What the box says about its own commissioning: who it is, whether the
/// network operator's control box is paired and heard, and the devices a
/// reduction may address.
///
/// Whole watts, so the same record is the same bytes wherever it is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Record {
    /// The version of this document: `1`.
    pub v: u8,
    /// The caller's own identifier for the work, echoed byte for byte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// When the box signed this, RFC 3339 UTC.
    pub issued_at: String,
    /// The box.
    #[serde(rename = "box")]
    pub this: RecordBox,
    /// The network operator's control box, as the box sees it now.
    pub control_box: RecordControlBox,
    /// `kept` where a restart restores this identity, `ephemeral` where the
    /// next start presents a new one.
    pub identity: &'static str,
    /// The § 14a module the household is billed under, as configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// The power a reduction may not go below, watts.
    pub minimum_w: u64,
    /// Each steuerbare Verbrauchseinrichtung.
    pub devices: Vec<RecordDevice>,
}

/// The box's own identity in a [`Record`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RecordBox {
    /// Its SKI, forty hexadecimal characters.
    pub ski: String,
    /// The SHIP ID it announces itself under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ship_id: Option<String>,
}

/// The control box in a [`Record`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RecordControlBox {
    /// `none`, `unpaired`, `silent`, `failsafe` or `heard` — see [`ControlBox`].
    pub state: &'static str,
    /// Its SKI, where exactly one is trusted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ski: Option<String>,
}

/// One steuerbare Verbrauchseinrichtung in a [`Record`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RecordDevice {
    /// `ladepunkt`, `waermepumpe`, `raumkuehlung` or `stromspeicher`.
    pub fallgruppe: &'static str,
    /// The letter `[A1 2.4.1]` lists it under.
    pub letter: &'static str,
    /// Its Netzanschlussleistung, watts.
    pub power_w: u64,
    /// What it alone would be owed under direct control, watts.
    pub minimum_direct_w: u64,
    /// The date of technical commissioning, where the configuration states it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commissioned: Option<String>,
    /// The assets that make it up, as the box names them. Left out of the
    /// record a QR code carries, where every octet is a module.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
}

impl Record {
    /// The record as a QR code carries it: without the box's own names for the
    /// devices, which nobody who was not at the box can use.
    #[must_use]
    pub fn for_a_code(mut self) -> Self {
        for device in &mut self.devices {
            device.assets.clear();
        }
        self
    }
}

/// The record a `sheet` and a `trust` check come to. `None` where the box has
/// no SKI to speak as.
///
/// Nothing is worked out here: the devices are the sheet's and the states are
/// the trust check's, so the record cannot say what the page does not.
#[must_use]
pub fn record(
    sheet: &Handover,
    trust: &TrustCheck,
    control_box_ski: Option<&str>,
    reference: Option<String>,
) -> Option<Record> {
    // Powers are configured in whole watts; rounding only removes float noise.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let watts = |w: f64| w.max(0.0).round() as u64;
    // One spelling for a SKI, whatever a screen shows: forty lower-case
    // hexadecimal characters, as SHIP writes it on the wire.
    let plain = |ski: &str| -> String {
        ski.chars()
            .filter(char::is_ascii_hexdigit)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    Some(Record {
        v: 1,
        reference,
        issued_at: sheet.generated_at.clone(),
        this: RecordBox {
            ski: plain(sheet.ski.as_deref()?),
            ship_id: sheet.ship_id.clone(),
        },
        control_box: RecordControlBox {
            state: trust.control_box.state,
            ski: control_box_ski.map(plain),
        },
        identity: trust.identity,
        module: sheet.module.clone(),
        minimum_w: watts(sheet.minimum_w),
        devices: sheet
            .devices
            .iter()
            .map(|d| RecordDevice {
                fallgruppe: d.fallgruppe,
                letter: d.letter,
                power_w: watts(d.power_w),
                minimum_direct_w: watts(d.minimum_direct_w),
                commissioned: d.commissioned.clone(),
                assets: d.assets.clone(),
            })
            .collect(),
    })
}

/// `record` as a JWS in compact serialisation, ES256, `[RFC 7515]`.
///
/// The protected header names the algorithm, [`RECORD_TYP`] and the SHA-256
/// thumbprint of `certificate` (`x5t#S256`); with `carry_certificate` it also
/// carries the certificate itself (`x5c`), so a verifier needs nothing else.
/// `sign` is handed the signing input and answers the 64-octet `r ‖ s`.
///
/// `None` where `sign` refuses.
pub fn jws(
    record: &Record,
    certificate: &[u8],
    carry_certificate: bool,
    sign: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Option<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    let thumbprint = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, certificate);
    let mut header = serde_json::Map::new();
    header.insert("alg".into(), "ES256".into());
    header.insert("typ".into(), RECORD_TYP.into());
    header.insert(
        "x5t#S256".into(),
        URL_SAFE_NO_PAD.encode(thumbprint.as_ref()).into(),
    );
    if carry_certificate {
        // `x5c` is the one place JOSE uses standard base64, `[RFC 7515 4.1.6]`.
        header.insert(
            "x5c".into(),
            serde_json::Value::Array(vec![STANDARD.encode(certificate).into()]),
        );
    }
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).ok()?),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(record).ok()?)
    );
    let signature = sign(input.as_bytes())?;
    Some(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// The longest record a code carries: what QR version 30 holds at the lowest
/// error correction, `[ISO/IEC 18004]` table 7.
///
/// Version 30 is 137 modules a side. Drawn 300 points wide on a phone that is
/// a little over two points a module — about the finest a second phone's
/// camera resolves off a screen at arm's length. A QR code can hold 2 953
/// octets (version 40), but nobody scans that from a hand. A record grows by
/// about 140 octets with each steuerbare Verbrauchseinrichtung, so a house
/// with up to seven fits and a larger installation is fetched instead.
pub const MAX_CODE_OCTETS: usize = 1_732;

/// A record too long to be scanned reliably; it has to be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooDense {
    /// How long the record is, octets.
    pub octets: usize,
}

/// `record` (a JWS) as a dense QR code, SVG: the lowest error correction, for
/// a code shown on a screen and scanned once, where every module counts.
///
/// # Errors
/// [`TooDense`] where the record is longer than [`MAX_CODE_OCTETS`].
pub fn qr_svg_dense(record: &str) -> Result<String, TooDense> {
    use qrcode::render::svg;
    let too_dense = TooDense {
        octets: record.len(),
    };
    if record.len() > MAX_CODE_OCTETS {
        return Err(too_dense);
    }
    let code = qrcode::QrCode::with_error_correction_level(record, qrcode::EcLevel::L)
        .map_err(|_| too_dense)?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(420, 420)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

// ── Trust ───────────────────────────────────────────────────────────────────

/// Whether the box is who it was, and whether its control box is still there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct TrustCheck {
    /// Optional installer contact for the household.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_contact: Option<String>,
    /// Seconds since the Energy Guard's last accepted LPC heartbeat, if observed.
    pub heartbeat_age_s: Option<u64>,
    /// Last control tick observed by the gateway.
    pub tick_at: Option<String>,
    /// Registry observations for configured Modbus and EEBUS assets.
    pub devices: Vec<crate::doctor::DeviceCheck>,
    /// `kept`: the store holds the certificate the box is presenting, so the
    /// SKI and fingerprint are the ones a restart restores. `ephemeral`: the box
    /// has no store, or the store holds another, and the next start presents a
    /// new identity. `none`: the box has no EEBUS identity.
    pub identity: &'static str,
    /// The box's SKI.
    pub ski: Option<String>,
    /// The SHA-256 fingerprint of its SHIP certificate.
    pub fingerprint: Option<String>,
    /// The SHA-256 fingerprint of the certificate the page is served with.
    pub https_fingerprint: Option<String>,
    /// The network operator's control box.
    pub control_box: ControlBox,
}

/// The network operator's control box, as this box sees it now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ControlBox {
    /// `none`: nothing here speaks to one. `unpaired`: no SKI is trusted yet.
    /// `silent`: one is trusted and no session is up. `failsafe`: its heartbeat
    /// stopped and the stored failsafe value applies. `heard`: in session and
    /// heartbeating.
    pub state: &'static str,
    /// How many SKIs this box trusts.
    pub trusted: usize,
    /// How many peers are waiting to be approved right now.
    pub waiting: usize,
}

/// What [`control_box`] needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlBoxFacts {
    /// Whether this box has an EEBUS identity and a control-box driver at all.
    pub configured: bool,
    /// How many SKIs it trusts.
    pub trusted: usize,
    /// How many peers are waiting.
    pub waiting: usize,
    /// Whether the control loop has ticked and the control box's link is down.
    pub silent: bool,
    /// Whether the stored failsafe value is in force.
    pub in_failsafe: bool,
}

/// The control box's state, worst first.
#[must_use]
pub fn control_box(facts: ControlBoxFacts) -> ControlBox {
    let state = if !facts.configured {
        "none"
    } else if facts.trusted == 0 {
        "unpaired"
    } else if facts.in_failsafe {
        "failsafe"
    } else if facts.silent {
        "silent"
    } else {
        "heard"
    };
    ControlBox {
        state,
        trusted: facts.trusted,
        waiting: facts.waiting,
    }
}

// ── Discovery ───────────────────────────────────────────────────────────────

/// Which hosts to ask.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "DiscoverAsk")]
pub struct Ask {
    /// Literal addresses on the household's network, e.g. `192.168.1.40`.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The Modbus unit identifier to ask for. 1 where absent.
    #[serde(default)]
    pub unit: Option<u8>,
    /// The port to ask on: 502 where absent, or 1502.
    #[serde(default)]
    pub port: Option<u16>,
}

/// Why an [`Ask`] was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AskError {
    /// More hosts than one discovery takes.
    #[error("at most {MAX_HOSTS} hosts in one discovery")]
    TooMany,
    /// Not a literal address.
    #[error("`{0}` is not an address: name each host by its IP address, without a port")]
    NotAnAddress(String),
    /// A port that is not one SunSpec is served on.
    #[error("port {0} is not one this box probes: 502 or 1502")]
    Port(u16),
    /// An address that is not on a household's network.
    #[error("`{0}` is not on a private network, and this box only looks on its own")]
    NotPrivate(IpAddr),
}

/// The addresses an [`Ask`] names, checked.
///
/// # Errors
/// [`AskError`] for too many hosts, a name, or an address off the household's
/// own network.
pub fn hosts(ask: &Ask) -> Result<Vec<IpAddr>, AskError> {
    if ask.hosts.len() > MAX_HOSTS {
        return Err(AskError::TooMany);
    }
    if let Some(port) = ask.port
        && !MODBUS_PORTS.contains(&port)
    {
        return Err(AskError::Port(port));
    }
    let mut out = Vec::new();
    for host in &ask.hosts {
        let ip: IpAddr = host
            .trim()
            .parse()
            .map_err(|_| AskError::NotAnAddress(host.escape_debug().to_string()))?;
        if !is_private(ip) {
            return Err(AskError::NotPrivate(ip));
        }
        if !out.contains(&ip) {
            out.push(ip);
        }
    }
    Ok(out)
}

/// Whether `ip` is on a network a household owns: RFC 1918, link-local,
/// loopback, or IPv6 unique-local.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// What a discovery found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
pub struct Discovered {
    /// Modbus: every configured device, then every host that was asked.
    pub modbus: Vec<ModbusDevice>,
    /// SHIP: every node announcing itself, then every configured one not seen.
    pub ship: Vec<ShipNode>,
}

/// One Modbus address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ModbusDevice {
    /// Where: an address for a host that was asked, `host:port` as configured
    /// for a device the box already manages.
    pub host: String,
    /// `configured`: the box manages it and hears it. `missing`: the box is
    /// configured for it and does not hear it. `found`: a SunSpec device
    /// answered and nothing is configured for it. `silent`: nothing SunSpec
    /// answered there.
    pub state: &'static str,
    /// The asset it is configured as.
    pub asset: Option<String>,
    /// What it says it is — a claim from the device, shown and not acted on.
    #[schemars(schema_with = "crate::openapi::sunspec_identity")]
    pub identity: Option<Identity>,
    /// Preview of one read-only driver addition for this site.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<InverterReview>,
}

/// A proposed PV driver block checked against the effective configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct InverterReview {
    /// Server-issued confirmation id when this file can be activated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    /// Proposed addition, containing only the selected device.
    pub toml: String,
    /// Refusal from the gateway's validator, if any.
    pub refusal: Option<String>,
    /// Modbus unit and endpoint used for the model walk.
    pub unit: u8,
    /// Literal socket address of the device.
    pub address: std::net::SocketAddr,
}

impl InverterReview {
    /// Review the exact TOML addition against a copy of the running settings.
    pub fn new(settings: &crate::Settings, address: std::net::SocketAddr, unit: u8) -> Self {
        let rating = settings.site.pv_ac_kw;
        let block = format!(
            "[[drivers]]\nkind = \"sunspec\"\nasset = \"pv\"\naddress = \"{address}\"\nunit = {unit}\nrating_kw = {rating:?}\n"
        );
        let mut candidate = settings.clone();
        let addition = toml::from_str::<crate::Settings>(&block);
        let refusal = match addition {
            Ok(addition) => {
                candidate.drivers.extend(addition.drivers);
                super::assemble(&candidate, None, OffsetDateTime::now_utc()).err().map(|error| match error {
                    super::StartError::Drivers(reason) => reason.to_string(),
                    _ => "The gateway refused the site configuration. Check it with hemsd run --check.".into(),
                })
            }
            Err(_) => Some("The proposed driver block could not be read.".into()),
        };
        Self {
            proposal: None,
            toml: block,
            refusal,
            unit,
            address,
        }
    }
}

/// One SHIP node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ShipNode {
    /// Its SKI, as it announces it — proved only once TLS has.
    pub ski: String,
    /// `configured`: the box is set up for it, or already trusts it.
    /// `missing`: configured, not announcing and not heard. `waiting`: its
    /// handshake is held for approval. `found`: announcing, and unknown here.
    pub state: &'static str,
    /// The asset it is configured as.
    pub asset: Option<String>,
    /// Its SHIP ID.
    pub ship_id: Option<String>,
    /// What it says its brand, type and model are.
    pub brand: Option<String>,
    /// See `brand`.
    pub device_type: Option<String>,
    /// See `brand`.
    pub model: Option<String>,
    /// Where it says it can be reached.
    pub addresses: Vec<String>,
    /// The port SHIP listens on there.
    pub port: Option<u16>,
}

/// What the box already knows, for [`discover`] to set what it finds against.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// The box's own SKI, which it would otherwise discover.
    pub own_ski: Option<String>,
    /// The SKIs it trusts.
    pub trusted: Vec<String>,
    /// The SKIs waiting to be approved.
    pub waiting: Vec<String>,
    /// The assets the control loop is not hearing from.
    pub silent: BTreeSet<AssetId>,
}

/// Look: the configured devices from what the box knows, the named hosts by
/// probe, and SHIP by listening for announcements.
pub async fn discover(
    commissioning: &Commissioning,
    known: &Known,
    asked: &[IpAddr],
    unit: u8,
    port: u16,
) -> Discovered {
    let mut modbus: Vec<ModbusDevice> = commissioning
        .modbus
        .iter()
        .map(|(asset, address)| ModbusDevice {
            host: address.clone(),
            state: if known.silent.contains(asset) {
                "missing"
            } else {
                "configured"
            },
            asset: Some(asset.to_string()),
            identity: None,
            review: None,
        })
        .collect();

    // A configured device is never probed — see the module note. Matched by the
    // host as it is written in the configuration: a device configured by name
    // and asked for by address is not recognised as the same one.
    let configured_hosts: BTreeSet<IpAddr> = commissioning
        .modbus
        .iter()
        .filter_map(|(_, address)| {
            address
                .parse::<std::net::SocketAddr>()
                .ok()
                .map(|socket| socket.ip())
        })
        .collect();
    let to_probe: Vec<IpAddr> = asked
        .iter()
        .copied()
        .filter(|ip| !configured_hosts.contains(ip))
        .collect();
    let probes = to_probe.iter().map(|ip| {
        let address = std::net::SocketAddr::new(*ip, port);
        inspect(address, unit)
    });
    let mut running = tokio::task::JoinSet::new();
    for (index, probing) in probes.enumerate() {
        running.spawn(async move { (index, probing.await) });
    }
    let mut answers: Vec<Option<Identity>> = vec![None; to_probe.len()];
    while let Some(joined) = running.join_next().await {
        if let Ok((index, identity)) = joined {
            answers[index] = identity;
        }
    }
    modbus.extend(
        to_probe
            .iter()
            .zip(answers)
            .map(|(ip, identity)| ModbusDevice {
                host: ip.to_string(),
                state: if identity.is_some() {
                    "found"
                } else {
                    "silent"
                },
                asset: None,
                identity,
                review: None,
            }),
    );

    let seen = tokio::task::spawn_blocking(|| browse(BROWSE_FOR))
        .await
        .unwrap_or_default();
    Discovered {
        modbus,
        ship: ship_nodes(commissioning, known, seen),
    }
}

/// Inspect a previously selected local endpoint under the discovery deadline.
pub(super) async fn inspect(address: std::net::SocketAddr, unit: u8) -> Option<Identity> {
    if !is_private(address.ip()) || !MODBUS_PORTS.contains(&address.port()) {
        return None;
    }
    tokio::time::timeout(PROBE_TIMEOUT, probe(address, unit))
        .await
        .ok()
        .flatten()
}

/// Ask one address whether a SunSpec device lives there. Reads only.
async fn probe(address: std::net::SocketAddr, unit: u8) -> Option<Identity> {
    let mut stream = tokio::net::TcpStream::connect(address).await.ok()?;
    let mut probe = Probe::new(unit);
    let mut buffer = Vec::with_capacity(260);
    while let Some(request) = probe.request() {
        stream.write_all(&request.encode()).await.ok()?;
        buffer.clear();
        let response = loop {
            if buffer.len() >= 7
                && (buffer[6] != unit || buffer[..2] != request.transaction.to_be_bytes())
            {
                return None;
            }
            match frame::decode(&buffer) {
                Ok(Some((response, _))) => break response,
                Ok(None) => {}
                Err(frame::FrameError::Exception { function, code }) => {
                    break frame::Response {
                        unit,
                        function,
                        transaction: request.transaction,
                        body: frame::ResponseBody::Exception { function, code },
                    };
                }
                Err(_) => return None,
            }
            let mut chunk = [0u8; 260];
            let read = stream.read(&mut chunk).await.ok()?;
            // Bounded: a Modbus TCP frame is at most 260 bytes, and whatever is
            // on the other end chose how much to send.
            if read == 0 || buffer.len() + read > 2 * 260 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        };
        if response.transaction != request.transaction {
            return None;
        }
        probe.answer(&response.body);
    }
    probe.found().cloned()
}

/// Every SHIP node that announced itself within `wait`.
fn browse(wait: Duration) -> Vec<eebus::mdns::Discovered> {
    let Ok(mdns) = eebus::mdns::Mdns::new() else {
        return Vec::new();
    };
    let Ok(browsing) = mdns.browse() else {
        return Vec::new();
    };
    let until = std::time::Instant::now() + wait;
    let mut seen: Vec<eebus::mdns::Discovered> = Vec::new();
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return seen;
        }
        match browsing.recv_timeout(left) {
            Some(eebus::mdns::BrowseEvent::Found(node)) => {
                seen.retain(|s| s.instance != node.instance);
                seen.push(node);
            }
            Some(eebus::mdns::BrowseEvent::Lost { instance }) => {
                seen.retain(|s| s.instance != instance);
            }
            Some(_) => {}
            None => return seen,
        }
    }
}

/// Set what was seen against what the box is configured for and trusts.
fn ship_nodes(
    commissioning: &Commissioning,
    known: &Known,
    seen: Vec<eebus::mdns::Discovered>,
) -> Vec<ShipNode> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let configured_as = |ski: &str| {
        commissioning
            .eebus
            .iter()
            .find(|(_, s)| same(s, ski))
            .map(|(asset, _)| asset.to_string())
    };
    let mut out: Vec<ShipNode> = Vec::new();
    for node in seen {
        let ski = node.ski.to_string();
        if known.own_ski.as_deref().is_some_and(|own| same(own, &ski))
            || out.iter().any(|n| same(&n.ski, &ski))
        {
            continue;
        }
        let asset = configured_as(&ski);
        let state = if asset.is_some() || known.trusted.iter().any(|t| same(t, &ski)) {
            "configured"
        } else if known.waiting.iter().any(|w| same(w, &ski)) {
            "waiting"
        } else {
            "found"
        };
        out.push(ShipNode {
            ski,
            state,
            asset,
            ship_id: Some(node.record.id.to_string()),
            brand: node.record.brand.as_deref().map(printable),
            device_type: node.record.device_type.as_deref().map(printable),
            model: node.record.model.as_deref().map(printable),
            addresses: node.addresses.iter().map(ToString::to_string).collect(),
            port: Some(node.port),
        });
    }
    for (asset, ski) in &commissioning.eebus {
        if out.iter().any(|n| same(&n.ski, ski)) {
            continue;
        }
        out.push(ShipNode {
            ski: ski.clone(),
            // Not announcing is not the same as gone: a device in session with
            // this box may have withdrawn its announcement, as this box does.
            state: if known.silent.contains(asset) {
                "missing"
            } else {
                "configured"
            },
            asset: Some(asset.to_string()),
            ship_id: None,
            brand: None,
            device_type: None,
            model: None,
            addresses: Vec::new(),
            port: None,
        });
    }
    out
}

/// A TXT value as something safe to show: printable ASCII, at most 64
/// characters. Whatever is on the network chose it.
fn printable(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(64)
        .collect()
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record for a house with six controllable devices, about as much as a
    /// household's connection carries.
    fn six_device_record() -> Record {
        let device = |fallgruppe, letter, asset: &str| RecordDevice {
            fallgruppe,
            letter,
            power_w: 11_000,
            minimum_direct_w: 4_200,
            commissioned: Some("2025-03-01".into()),
            assets: vec![asset.into()],
        };
        Record {
            v: 1,
            reference: Some("x".repeat(MAX_REFERENCE)),
            issued_at: "2026-11-19T20:03:11Z".into(),
            this: RecordBox {
                ski: "f806f2954b318f815d785c934c0a5d071dc03e08".into(),
                ship_id: Some("hems_hems-1".into()),
            },
            control_box: RecordControlBox {
                state: "failsafe",
                ski: Some("8e50225d7685cca722e7e4e6dafc1df0b2f04c4e".into()),
            },
            identity: "ephemeral",
            module: Some("modul1".into()),
            minimum_w: 16_800,
            devices: vec![
                device("ladepunkt", "a", "wallbox-garage"),
                device("ladepunkt", "a", "wallbox-carport"),
                device("waermepumpe", "b", "waermepumpe"),
                device("waermepumpe", "b", "warmwasser"),
                device("raumkuehlung", "c", "klimaanlage"),
                device("stromspeicher", "d", "speicher"),
            ],
        }
    }

    #[test]
    fn the_code_is_measured_against_what_a_phone_reads_off_a_screen() {
        // Measured, not assumed. A phone reads a code off another screen
        // comfortably up to about version 26 (121 modules a side): 1 367 octets
        // at the lowest error correction. Past that it still scans, with care.
        const COMFORTABLE: usize = 1_367;
        let identity = eebus::cert::self_signed(eebus::cert::CertParams::new("hems_hems-1"))
            .expect("a certificate");
        let certificate = identity.certificate_der();
        let sign = |_: &[u8]| Some(vec![0u8; 64]);
        let modules = |text: &str| {
            qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::L)
                .expect("it fits a QR code")
                .width()
        };

        // Four devices — a charge point, a heat pump, cooling and a battery —
        // with the longest reference: comfortable without the certificate and
        // the box's own names, and not with them.
        let mut four = six_device_record();
        four.devices.truncate(4);
        let whole = jws(&four, certificate, true, sign).expect("a record");
        let code = jws(&four.clone().for_a_code(), certificate, false, sign).expect("a record");
        assert!(
            whole.len() > COMFORTABLE,
            "with its certificate the record is {} octets; if that now fits, carry it",
            whole.len()
        );
        assert!(
            code.len() <= COMFORTABLE,
            "the code is {} octets",
            code.len()
        );
        assert!(modules(&code) <= 121, "{} modules a side", modules(&code));

        // Six still draw, denser: version 30 at most.
        let six =
            jws(&six_device_record().for_a_code(), certificate, false, sign).expect("a record");
        assert!(
            modules(&six) <= 137,
            "{} modules a side for {} octets",
            modules(&six),
            six.len()
        );
        assert!(qr_svg_dense(&six).is_ok());
    }

    #[test]
    fn a_code_carries_seven_devices_and_refuses_the_eighth() {
        // The bound a field tool is told: with the longest reference, a house
        // with seven steuerbare Verbrauchseinrichtungen is still a code, and a
        // larger installation is a download. If a field is added to the record
        // this moves, and the documented number has to move with it.
        let sign = |_: &[u8]| Some(vec![0u8; 64]);
        let of = |devices: usize| {
            let mut record = six_device_record();
            let one = record.devices[0].clone();
            record.devices.resize(devices, one);
            jws(&record.for_a_code(), b"certificate", false, sign).expect("a record")
        };
        let seven = of(7);
        assert!(seven.len() <= MAX_CODE_OCTETS, "{} octets", seven.len());
        let drawn = qrcode::QrCode::with_error_correction_level(&seven, qrcode::EcLevel::L)
            .expect("it fits");
        assert!(drawn.width() <= 137, "{} modules a side", drawn.width());
        assert!(qr_svg_dense(&seven).is_ok());

        let eight = of(8);
        assert_eq!(
            qr_svg_dense(&eight),
            Err(TooDense {
                octets: eight.len()
            })
        );
        // Twenty would not fit any QR code at all; the fetch has no such limit.
        assert!(of(20).len() > 2_953);
    }

    #[test]
    fn a_reference_is_printable_ascii_without_a_space() {
        for good in ["A", "WO-2026/000417#a~b", &"x".repeat(MAX_REFERENCE)] {
            assert!(is_reference(good), "{good:?}");
        }
        for bad in [
            "",
            "has space",
            "ä",
            "tab\there",
            "line\nbreak",
            &"x".repeat(MAX_REFERENCE + 1),
        ] {
            assert!(!is_reference(bad), "{bad:?}");
        }
    }

    #[test]
    fn every_signing_input_begins_with_the_records_own_header() {
        // What keeps this signature apart from any other the key makes: the
        // input is text, and its first characters are this fixed header's.
        let mut seen = Vec::new();
        let _ = jws(&six_device_record(), b"certificate", false, |input| {
            seen = input.to_vec();
            Some(vec![0u8; 64])
        });
        let text = String::from_utf8(seen).expect("ASCII");
        assert!(text.starts_with("eyJhbGciOiJFUzI1NiIsInR5cCI6ImhlbXMtY29tbWlzc2lvbmluZytqd3Mi"));
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        );
    }

    const SKI: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn the_ship_code_is_one_ship_itself_reads_back() {
        let fingerprint = "ab".repeat(32);
        let payload = ship_payload(SKI, "hems_box", Some(&fingerprint)).expect("a payload");
        assert!(payload.starts_with("SHIP;SKI:"), "{payload}");
        assert!(payload.ends_with("ENDSHIP;"), "{payload}");
        let read: eebus::ship::ShipQr = payload.parse().expect("SHIP's own parser reads it");
        assert_eq!(read.ski.to_string().to_lowercase(), SKI);
        assert_eq!(
            read.id.map(|id| id.to_string()).as_deref(),
            Some("hems_box")
        );
        assert_eq!(
            read.certificate_fingerprint.map(|f| f.to_hex()),
            Some(fingerprint.to_ascii_uppercase()),
            "however the caller happened to spell it"
        );
    }

    #[test]
    fn a_qr_code_is_an_svg_and_nothing_executable() {
        let svg = qr_svg(&sign_in_link("https://hems.local", "token")).expect("a code");
        assert!(svg.contains("<svg"), "{svg}");
        assert!(
            !svg.contains("<script"),
            "an SVG may carry script; this one does not"
        );
        assert!(!svg.contains("token"), "the text is modules, not markup");
    }

    #[test]
    fn an_origin_is_a_scheme_and_a_host_and_nothing_a_scanner_would_follow_elsewhere() {
        for good in [
            "https://hems.local",
            "http://192.168.1.5:8080",
            "https://[fd00::1]:8443",
        ] {
            assert!(is_origin(good), "{good}");
        }
        for bad in [
            "hems.local",
            "javascript:alert(1)",
            "https://hems.local/path",
            "https://user@evil.example",
            "https://hems.local?x=1",
            "https://hems.local#t=x",
            "https://a b",
            "https://",
        ] {
            assert!(!is_origin(bad), "{bad}");
        }
    }

    #[test]
    fn only_literal_private_addresses_are_ever_probed() {
        let ask = |hosts: &[&str]| Ask {
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
            unit: None,
            port: None,
        };
        assert_eq!(
            hosts(&ask(&["192.168.1.40", "10.0.0.7", "192.168.1.40"])).map(|h| h.len()),
            Ok(2),
            "and a host named twice is asked once"
        );
        assert!(matches!(
            hosts(&ask(&["inverter.local"])),
            Err(AskError::NotAnAddress(_))
        ));
        assert!(matches!(
            hosts(&ask(&["192.168.1.40:22"])),
            Err(AskError::NotAnAddress(_))
        ));
        assert!(matches!(
            hosts(&ask(&["8.8.8.8"])),
            Err(AskError::NotPrivate(_))
        ));
        assert!(matches!(
            hosts(&ask(&["2001:db8::1"])),
            Err(AskError::NotPrivate(_))
        ));
        let on = |port| Ask {
            hosts: vec!["192.168.1.40".into()],
            unit: None,
            port: Some(port),
        };
        assert!(hosts(&on(1502)).is_ok(), "where Kostal serves SunSpec");
        assert_eq!(hosts(&on(22)), Err(AskError::Port(22)));
        let many: Vec<String> = (0..=MAX_HOSTS).map(|i| format!("10.0.0.{i}")).collect();
        assert_eq!(
            hosts(&Ask {
                hosts: many,
                unit: None,
                port: None,
            }),
            Err(AskError::TooMany)
        );
    }

    #[test]
    fn the_control_box_is_reported_by_its_worst_fact() {
        let facts = ControlBoxFacts {
            configured: true,
            trusted: 1,
            waiting: 0,
            silent: false,
            in_failsafe: false,
        };
        assert_eq!(control_box(facts).state, "heard");
        assert_eq!(
            control_box(ControlBoxFacts {
                silent: true,
                ..facts
            })
            .state,
            "silent"
        );
        assert_eq!(
            control_box(ControlBoxFacts {
                in_failsafe: true,
                silent: true,
                ..facts
            })
            .state,
            "failsafe"
        );
        assert_eq!(
            control_box(ControlBoxFacts {
                trusted: 0,
                ..facts
            })
            .state,
            "unpaired"
        );
        assert_eq!(
            control_box(ControlBoxFacts {
                configured: false,
                ..facts
            })
            .state,
            "none"
        );
    }

    #[test]
    fn review_validates_only_the_proposed_addition_and_keeps_the_source_settings() {
        let mut settings: crate::Settings =
            toml::from_str(include_str!("../../hemsd.example.toml")).unwrap();
        let address = "127.0.0.1:1502".parse().unwrap();
        let duplicate = InverterReview::new(&settings, address, 7);
        assert!(duplicate.refusal.unwrap().contains("two drivers"));
        settings.drivers.retain(|driver| driver.asset() != "pv");
        let before = settings.clone();
        let review = InverterReview::new(&settings, address, 7);
        assert_eq!(review.refusal, None);
        assert_eq!(settings, before);
        assert_eq!(review.unit, 7);
        assert_eq!(review.address, address);
        assert!(review.toml.contains("unit = 7"));
        assert!(!review.toml.contains("[api]"));
    }

    /// A SunSpec device on loopback: the marker at 40 000 and a common model.
    async fn sunspec_device() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 12];
            while stream.read_exact(&mut request).await.is_ok() {
                let at = u16::from_be_bytes([request[8], request[9]]);
                let count = u16::from_be_bytes([request[10], request[11]]);
                let mut registers = vec![0u16; usize::from(count)];
                if at == 40_000 {
                    registers = vec![0x5375, 0x6e53];
                } else if at == 40_002 {
                    registers[0] = 1;
                    registers[1] = 66;
                    for (i, pair) in b"SMA\0".chunks(2).enumerate() {
                        registers[2 + i] = u16::from_be_bytes([pair[0], pair[1]]);
                    }
                }
                let bytes = u8::try_from(registers.len() * 2).unwrap();
                let mut out = vec![request[0], request[1], 0, 0];
                out.extend_from_slice(&(u16::from(bytes) + 3).to_be_bytes());
                out.extend_from_slice(&[request[6], 0x03, bytes]);
                for r in registers {
                    out.extend_from_slice(&r.to_be_bytes());
                }
                stream.write_all(&out).await.unwrap();
            }
        });
        address
    }

    #[tokio::test]
    async fn a_sunspec_device_on_the_network_names_itself() {
        let address = sunspec_device().await;
        let identity = probe(address, 1).await.expect("it answers");
        assert_eq!(identity.manufacturer, "SMA");
        assert_eq!(identity.base, 40_000);
        assert_eq!(identity.inverter_model, None);
        assert!(!identity.has_immediate_controls);
    }

    #[tokio::test]
    async fn an_address_with_nothing_on_it_is_silent_rather_than_an_error() {
        // A listener that is dropped at once: the port is closed.
        let address = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap()
        };
        assert_eq!(probe(address, 1).await, None);
    }

    /// Some inverters serve one Modbus client. The box's driver is that
    /// client, so a discovery that names the same host must not dial it.
    #[tokio::test]
    async fn a_host_the_box_already_manages_is_not_dialled_a_second_time() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let dialled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = std::sync::Arc::clone(&dialled);
        tokio::spawn(async move {
            if listener.accept().await.is_ok() {
                seen.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let commissioning = Commissioning {
            modbus: vec![(AssetId::new("pv").unwrap(), address.to_string())],
            ..Commissioning::default()
        };
        let found = discover(
            &commissioning,
            &Known::default(),
            &[address.ip()],
            1,
            address.port(),
        )
        .await;
        assert_eq!(found.modbus.len(), 1, "{:?}", found.modbus);
        assert_eq!(found.modbus[0].state, "configured");
        assert!(
            !dialled.load(std::sync::atomic::Ordering::SeqCst),
            "the box opened a second connection to a device it manages"
        );
    }

    #[test]
    fn a_configured_device_is_reported_from_what_the_box_hears_and_never_probed() {
        let wallbox = AssetId::new("wallbox").unwrap();
        let commissioning = Commissioning {
            eebus: vec![(wallbox.clone(), SKI.to_owned())],
            ..Commissioning::default()
        };
        let known = Known {
            silent: BTreeSet::from([wallbox]),
            ..Known::default()
        };
        let nodes = ship_nodes(&commissioning, &known, Vec::new());
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].state, "missing");
        let heard = ship_nodes(&commissioning, &Known::default(), Vec::new());
        assert_eq!(heard[0].state, "configured");
    }
}
