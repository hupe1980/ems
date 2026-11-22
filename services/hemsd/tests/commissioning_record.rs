//! The signed commissioning record, as whoever ordered the installation meets
//! it: fetched or scanned, then verified with nothing but the certificate.
//!
//! Verification here deliberately shares no code with the signer: the public
//! key is cut out of the certificate's DER by its `SubjectPublicKeyInfo`
//! prefix, the signature is checked by `aws-lc-rs`'s verifier, and the SKI is
//! `eebus`'s own reading of the certificate.

use std::sync::Arc;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hemsd::runtime::access::LocalAccess;
use hemsd::runtime::api::{HomeSources, Local};
use hemsd::runtime::handover::Commissioning;
use hemsd::runtime::ship::{self, ShipSettings, Trust};
use tokio::sync::Mutex;

const TOKEN: &str = "a-token-only-this-household-has";
const STEUERBOX: &str = "7b10e3d2a4c95f8812d0b6e3c1a47f90d25ec94f";

/// A box with a SHIP identity and one trusted control box, on a loopback port.
/// Answers its address, its access layer and its SKI as SHIP writes it.
async fn a_box(with_identity: bool) -> (String, LocalAccess, String) {
    let household = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household is a valid site");
    let site = household.site.clone();
    let name = site.id.to_string();
    let access = LocalAccess::for_testing(&name, TOKEN);
    let settings = ShipSettings {
        ship_id: "hems_test".into(),
        trust: vec![STEUERBOX.into()],
        ..ShipSettings::default()
    };
    let (node, ski, kept) = ship::identity(&settings, None, time::OffsetDateTime::now_utc())
        .await
        .expect("an identity");
    let fingerprint = node.fingerprint().to_hex();
    let trust = Trust::new(Arc::new(node), None, settings.ship_id.clone(), kept);
    let local = Local::new(
        Arc::new(Mutex::new(hemsd::runtime::control::Status::default())),
        name,
        with_identity.then(|| ski.to_display_string()),
        hemsd::runtime::overrides::Overrides::new(),
        with_identity.then_some(trust),
        access.clone(),
        None,
    )
    .with_home(HomeSources {
        site: Some(Arc::new(site)),
        fingerprint: Some(fingerprint),
        ..HomeSources::default()
    })
    .with_commissioning(Commissioning {
        ship_id: Some("hems_test".into()),
        control_box: ems_core::prelude::AssetId::new("netzanschluss").ok(),
        ..Commissioning::default()
    });
    let router = hemsd::runtime::surfaces_over(local, None, access.clone(), false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("its address").to_string();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (address, access, ski.to_string().to_ascii_lowercase())
}

async fn get(address: &str, path: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("http://{address}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .expect("the box to answer")
}

fn json(part: &str) -> serde_json::Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).expect("base64url")).expect("JSON")
}

/// The uncompressed P-256 point in a certificate, found by the fixed
/// `SubjectPublicKeyInfo` prefix for id-ecPublicKey on prime256v1.
fn public_key_of(certificate: &[u8]) -> &[u8] {
    const SPKI: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    let at = certificate
        .windows(SPKI.len())
        .position(|w| w == SPKI)
        .expect("a P-256 key in the certificate");
    &certificate[at + SPKI.len()..at + SPKI.len() + 65]
}

fn verifies(jws: &str, certificate: &[u8]) -> bool {
    let (input, signature) = jws.rsplit_once('.').expect("three parts");
    let signature = URL_SAFE_NO_PAD.decode(signature).expect("base64url");
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public_key_of(certificate))
        .verify(input.as_bytes(), &signature)
        .is_ok()
}

#[tokio::test]
async fn the_record_verifies_with_nothing_but_what_it_carries() {
    let (address, _, ski) = a_box(true).await;
    let reference = "WO-2026/000417#a~b";
    let response = get(
        &address,
        &format!("/v1/commissioning?reference={}", "WO-2026%2F000417%23a~b"),
        TOKEN,
    )
    .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/jose");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let jws = response.text().await.expect("a body");
    let parts: Vec<&str> = jws.split('.').collect();
    assert_eq!(parts.len(), 3, "a JWS in compact serialisation");

    let header = json(parts[0]);
    assert_eq!(header["alg"], "ES256");
    assert_eq!(header["typ"], "hems-commissioning+jws");
    let certificate = STANDARD
        .decode(header["x5c"][0].as_str().expect("a certificate"))
        .expect("standard base64, as RFC 7515 4.1.6 asks");
    let thumbprint = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &certificate);
    assert_eq!(
        header["x5t#S256"],
        URL_SAFE_NO_PAD.encode(thumbprint.as_ref())
    );

    assert!(
        verifies(&jws, &certificate),
        "the signature is the certificate's key's"
    );

    // The key that signed is the one behind the SKI the operator was given.
    let of_certificate = eebus::cert::ski_from_der(&certificate).expect("a SKI");
    let payload = json(parts[1]);
    assert_eq!(payload["box"]["ski"], ski);
    assert_eq!(
        payload["box"]["ski"],
        of_certificate.to_string().to_ascii_lowercase()
    );

    assert_eq!(payload["v"], 1);
    assert_eq!(payload["reference"], reference, "echoed byte for byte");
    assert_eq!(payload["box"]["ship_id"], "hems_test");
    assert_eq!(payload["control_box"]["ski"], STEUERBOX);
    // Trusted, and no control loop has heard it yet.
    assert_eq!(payload["control_box"]["state"], "silent");
    assert_eq!(payload["identity"], "ephemeral", "this box keeps no store");
    assert!(
        payload["issued_at"]
            .as_str()
            .expect("an instant")
            .ends_with('Z')
    );

    // The devices and the minimum are the handover sheet's, in whole watts.
    let sheet: serde_json::Value = get(&address, "/v1/handover", TOKEN)
        .await
        .json()
        .await
        .expect("the sheet");
    assert!(payload["minimum_w"].is_u64());
    assert_eq!(payload["minimum_w"].as_f64(), sheet["minimum_w"].as_f64());
    let devices = payload["devices"].as_array().expect("devices");
    let printed = sheet["devices"].as_array().expect("devices");
    assert!(!devices.is_empty());
    assert_eq!(devices.len(), printed.len());
    for (signed, printed) in devices.iter().zip(printed) {
        assert_eq!(signed["fallgruppe"], printed["fallgruppe"]);
        assert_eq!(signed["letter"], printed["letter"]);
        assert_eq!(signed["assets"], printed["assets"]);
        assert!(signed["power_w"].is_u64());
        assert_eq!(signed["power_w"].as_f64(), printed["power_w"].as_f64());
        assert_eq!(
            signed["minimum_direct_w"].as_f64(),
            printed["minimum_direct_w"].as_f64()
        );
    }
}

#[tokio::test]
async fn a_changed_record_no_longer_verifies() {
    let (address, _, _) = a_box(true).await;
    let jws = get(&address, "/v1/commissioning?reference=A1", TOKEN)
        .await
        .text()
        .await
        .expect("a body");
    let parts: Vec<&str> = jws.split('.').collect();
    let certificate = STANDARD
        .decode(json(parts[0])["x5c"][0].as_str().expect("a certificate"))
        .expect("base64");
    assert!(verifies(&jws, &certificate));

    let mut payload = json(parts[1]);
    payload["control_box"]["state"] = "heard".into();
    let forged = format!(
        "{}.{}.{}",
        parts[0],
        URL_SAFE_NO_PAD.encode(payload.to_string()),
        parts[2]
    );
    assert!(!verifies(&forged, &certificate));
}

#[tokio::test]
async fn without_a_reference_the_record_carries_none() {
    let (address, _, _) = a_box(true).await;
    let jws = get(&address, "/v1/commissioning", TOKEN)
        .await
        .text()
        .await
        .expect("a body");
    let payload = json(jws.split('.').nth(1).expect("a payload"));
    assert!(payload.get("reference").is_none());
}

#[tokio::test]
async fn a_reference_that_is_not_plain_text_is_refused() {
    let (address, _, _) = a_box(true).await;
    let long = "x".repeat(65);
    for bad in [
        "has%20space",
        "",
        "umlaut-%C3%A4",
        "line%0Abreak",
        long.as_str(),
    ] {
        for path in ["/v1/commissioning", "/v1/qr/commissioning"] {
            let status = get(&address, &format!("{path}?reference={bad}"), TOKEN)
                .await
                .status();
            assert_eq!(
                status,
                reqwest::StatusCode::BAD_REQUEST,
                "{path} took {bad:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_manager_is_handed_no_record() {
    let (address, access, _) = a_box(true).await;
    let manager = access
        .connect_manager("aggregator-nord")
        .await
        .expect("a manager can be connected");
    for path in ["/v1/commissioning", "/v1/qr/commissioning"] {
        let status = get(&address, path, &manager).await.status();
        assert_eq!(
            status,
            reqwest::StatusCode::FORBIDDEN,
            "a manager reached {path}"
        );
    }
}

#[tokio::test]
async fn a_box_with_no_identity_signs_nothing() {
    let (address, _, _) = a_box(false).await;
    for path in ["/v1/commissioning", "/v1/qr/commissioning"] {
        let status = get(&address, path, TOKEN).await.status();
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn the_code_is_the_same_record_without_its_certificate_and_small_enough_to_scan() {
    let (address, _, _) = a_box(true).await;
    let response = get(&address, "/v1/qr/commissioning?reference=WO-417", TOKEN).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/svg+xml");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.text().await.expect("a body").starts_with("<?xml"));
}
