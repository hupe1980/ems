//! The household's own surface, as a browser and an installer meet it.
//!
//! Signing in, what commissioning serves and who may ask for it, and the same
//! surfaces over HTTPS with the certificate pinned — an S2 manager included.
//! Every box here is assembled by `runtime::surfaces_over`, the arrangement
//! `main` serves, because the security properties are layers over that assembly
//! and a test that built its own would pass while the product shipped open.

use std::sync::Arc;

use hems_core::prelude::{Asset, AssetId, Site};
use hemsd::drivers::Registry;
use hemsd::runtime::access::LocalAccess;
use hemsd::runtime::api::{HomeSources, Local};
use hemsd::runtime::handover::Commissioning;
use hemsd::runtime::s2::{Cem, S2Settings, Surface};
use s2_kit::connect::tls::{Fingerprint, TlsClient, TlsPolicy};
use s2_kit::io::{Driver, WebSocket, WebSocketOptions};
use s2_kit::session::{CemConfig, CemEvent, CemSession};
use s2_kit::types::Timestamp;
use s2_kit::types::common::ControlType;
use tokio::sync::Mutex;

const TOKEN: &str = "a-token-only-this-household-has";
const SKI: &str = "7b10e3d2a4c95f8812d0b6e3c1a47f90d25ec94f";

/// The router `main` serves, for the reference household.
fn surfaces(https_fingerprint: Option<String>) -> (axum::Router, LocalAccess, Site) {
    let household = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household is a valid site");
    let site = household.site.clone();
    let name = site.id.to_string();
    let access = LocalAccess::for_testing(&name, TOKEN).remembering(Some(Arc::new(Mutex::new(
        hemsd::store::Store::in_memory().expect("a store"),
    ))));
    let https = https_fingerprint.is_some();
    let local = Local::new(
        Arc::new(Mutex::new(hemsd::runtime::control::Status::default())),
        name,
        Some(SKI.into()),
        hemsd::runtime::overrides::Overrides::new(),
        None,
        access.clone(),
        None,
    )
    .with_home(HomeSources {
        site: Some(Arc::new(site.clone())),
        fingerprint: Some("ab".repeat(32)),
        https_fingerprint,
        ..HomeSources::default()
    })
    .with_commissioning(Commissioning {
        ship_id: Some("hems_test".into()),
        ..Commissioning::default()
    });
    let surface = Surface::new(
        Arc::new(site.clone()),
        Arc::new(Mutex::new(Registry::new())),
        Cem::new(),
        S2Settings {
            enabled: true,
            ..S2Settings::default()
        },
    );
    (
        hemsd::runtime::surfaces_over(local, Some(surface), access.clone(), https),
        access,
        site,
    )
}

/// The box on a loopback port, in clear.
async fn a_box() -> (String, LocalAccess) {
    let (router, access, _) = surfaces(None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("its address").to_string();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (address, access)
}

/// The box serving HTTPS the way `main` does: `runtime::tls`'s certificate
/// through `hems_service::Server::with_tls`.
struct TlsBox {
    port: u16,
    fingerprint: String,
    site: Site,
    _stop: hems_service::shutdown::ShutdownTrigger,
}

async fn a_box_over_tls() -> TlsBox {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let (config, fingerprint) =
        hemsd::runtime::tls::server_config(None).expect("a certificate of its own");
    let (router, _, site) = surfaces(Some(fingerprint.clone()));
    // A port nobody holds: bound, read and released for the server to take.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("a port")
        .port();
    let settings = hems_service::Settings {
        listen: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        shutdown_grace_s: 1,
        ..hems_service::Settings::default()
    };
    let (signal, trigger) = hems_service::Shutdown::channel();
    let server = hems_service::Server::new(
        hems_service::identity!(),
        settings,
        hems_service::Health::new(),
        router,
    )
    .with_tls(config);
    tokio::spawn(async move {
        let _ = server.run_until(signal).await;
    });
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    TlsBox {
        port,
        fingerprint,
        site,
        _stop: trigger,
    }
}

/// An HTTP client that trusts exactly one certificate, by fingerprint.
fn pinned(fingerprint: &str) -> reqwest::Client {
    let policy = TlsPolicy::Pinned(Fingerprint::parse(fingerprint).expect("a fingerprint"));
    let tls = TlsClient::new(&policy).expect("a pinned client");
    reqwest::Client::builder()
        .tls_backend_preconfigured((*tls.config()).clone())
        .build()
        .expect("a client")
}

fn cookie_of(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("a cookie")
        .to_str()
        .expect("printable")
        .to_owned()
}

/// `name=value`, as a browser would send it back.
fn pair(cookie: &str) -> String {
    cookie.split(';').next().unwrap_or_default().to_owned()
}

async fn sign_in(client: &reqwest::Client, base: &str, token: &str) -> reqwest::Response {
    client
        .post(format!("{base}/v1/session"))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .expect("the box to answer")
}

// ── Signing in ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn signing_in_hands_back_a_cookie_no_script_can_read() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let response = sign_in(&client, &base, TOKEN).await;
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    let cookie = cookie_of(&response);
    assert!(cookie.starts_with("hems_session="), "{cookie}");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(cookie.contains(attribute), "{attribute} missing: {cookie}");
    }
    assert!(
        !cookie.contains("Secure"),
        "a box in clear on loopback cannot set a cookie a browser would only send over TLS"
    );

    // And the cookie alone opens a read.
    let status = client
        .get(format!("{base}/v1/status"))
        .header(reqwest::header::COOKIE, pair(&cookie))
        .send()
        .await
        .expect("the box to answer")
        .status();
    assert_eq!(status, reqwest::StatusCode::OK);
}

#[tokio::test]
async fn only_the_households_own_token_signs_in() {
    let (address, access) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let manager = access
        .connect_manager("aggregator-nord")
        .await
        .expect("a manager can be connected");
    for (who, token) in [("a manager", manager.as_str()), ("a stranger", "guessed")] {
        let response = sign_in(&client, &base, token).await;
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{who} signed in"
        );
        assert!(
            response
                .headers()
                .get(reqwest::header::SET_COOKIE)
                .is_none(),
            "{who} was handed a cookie"
        );
    }
}

/// A browser sends a cookie on its own, so a write that rides on one must also
/// carry a body type a cross-site form cannot send.
#[tokio::test]
async fn a_write_on_the_cookie_needs_a_json_body() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let cookie = pair(&cookie_of(&sign_in(&client, &base, TOKEN).await));

    let as_a_form = client
        .delete(format!("{base}/v1/overrides"))
        .header(reqwest::header::COOKIE, &cookie)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(as_a_form.status(), reqwest::StatusCode::UNAUTHORIZED);

    let bare = client
        .delete(format!("{base}/v1/overrides"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(bare.status(), reqwest::StatusCode::UNAUTHORIZED);

    let as_the_page = client
        .delete(format!("{base}/v1/overrides"))
        .header(reqwest::header::COOKIE, &cookie)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .send()
        .await
        .expect("the box to answer");
    // Past the content-type rule: nothing was in force, so the answer is the
    // route's own 404 rather than the layer's refusal of a form-shaped write.
    assert_eq!(
        as_the_page.status(),
        reqwest::StatusCode::NOT_FOUND,
        "{}",
        as_the_page.status()
    );
}

#[tokio::test]
async fn signing_out_takes_the_cookie_back() {
    let (address, _) = a_box().await;
    let response = reqwest::Client::new()
        .delete(format!("http://{address}/v1/session"))
        .send()
        .await
        .expect("the box to answer");
    let cookie = cookie_of(&response);
    assert!(cookie.starts_with("hems_session=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");
}

// ── Commissioning ───────────────────────────────────────────────────────────

/// The codes carry the box's identities and one of them its credential; the
/// sheet says who the box obeys; discovery opens sockets from the box's place
/// on the network. None of that is a manager's.
#[tokio::test]
async fn a_manager_reaches_nothing_of_commissioning() {
    let (address, access) = a_box().await;
    let base = format!("http://{address}");
    let manager = access
        .connect_manager("aggregator-nord")
        .await
        .expect("a manager can be connected");
    let client = reqwest::Client::new();
    for (what, request) in [
        ("the SHIP code", client.get(format!("{base}/v1/qr/ski"))),
        (
            "the fingerprint code",
            client.get(format!("{base}/v1/qr/fingerprint")),
        ),
        (
            "the sign-in code",
            client.get(format!("{base}/v1/qr/signin")),
        ),
        (
            "the handover sheet",
            client.get(format!("{base}/v1/handover")),
        ),
        ("the trust check", client.get(format!("{base}/v1/trust"))),
        (
            "a discovery",
            client
                .post(format!("{base}/v1/discover"))
                .json(&serde_json::json!({ "hosts": ["127.0.0.1"] })),
        ),
    ] {
        let status = request
            .bearer_auth(&manager)
            .send()
            .await
            .expect("the box to answer")
            .status();
        assert_eq!(
            status,
            reqwest::StatusCode::FORBIDDEN,
            "a manager reached {what}"
        );
    }
}

/// The cookie is `HttpOnly` so nothing running in the page can read the
/// credential. A code that carried it to any signed-in request would undo that.
#[tokio::test]
async fn the_sign_in_code_answers_only_somebody_who_already_holds_the_token() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let cookie = pair(&cookie_of(&sign_in(&client, &base, TOKEN).await));

    let on_the_cookie = client
        .get(format!("{base}/v1/qr/signin"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(on_the_cookie.status(), reqwest::StatusCode::FORBIDDEN);

    let with_the_token = client
        .get(format!("{base}/v1/qr/signin?origin=https://hems.local"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(with_the_token.status(), reqwest::StatusCode::OK);
    let headers = with_the_token.headers().clone();
    assert_eq!(headers[reqwest::header::CONTENT_TYPE], "image/svg+xml");
    assert_eq!(headers[reqwest::header::CACHE_CONTROL], "no-store");
    let svg = with_the_token.text().await.expect("a body");
    assert!(svg.contains("<svg"));
    assert!(!svg.contains(TOKEN), "the token is modules, never markup");

    for origin in ["https://hems.local/elsewhere", "javascript:alert(1)"] {
        let status = client
            .get(format!(
                "{base}/v1/qr/signin?origin={}",
                origin.replace(':', "%3A").replace('/', "%2F")
            ))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("the box to answer")
            .status();
        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{origin}");
    }
}

#[tokio::test]
async fn the_identity_codes_are_served_to_the_signed_in_page() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let cookie = pair(&cookie_of(&sign_in(&client, &base, TOKEN).await));
    for what in ["ski", "fingerprint"] {
        let response = client
            .get(format!("{base}/v1/qr/{what}"))
            .header(reqwest::header::COOKIE, &cookie)
            .send()
            .await
            .expect("the box to answer");
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{what}");
        assert_eq!(
            response.headers()[reqwest::header::CONTENT_TYPE],
            "image/svg+xml"
        );
    }
    let unknown = client
        .get(format!("{base}/v1/qr/token"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
}

/// The sheet states the same minimum the box defaults its failsafe to, from the
/// same classification — two derivations would be two numbers on one visit.
#[tokio::test]
async fn the_handover_sheet_states_what_the_operators_registration_asks_for() {
    let (address, _) = a_box().await;
    let sheet: serde_json::Value = reqwest::Client::new()
        .get(format!("http://{address}/v1/handover"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer")
        .json()
        .await
        .expect("a document");
    assert_eq!(sheet["ski"], SKI);
    assert_eq!(sheet["ship_id"], "hems_test");
    assert_eq!(sheet["control"], "ems");

    let site = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household")
        .site;
    let classified = hems_grid::classify_at(&site.assets, time::OffsetDateTime::now_utc());
    let expected =
        hems_grid::para14a::minimum_power(&classified, hems_grid::para14a::ControlMode::Ems)
            .max(hems_grid::para14a::MINDESTLEISTUNG)
            .get();
    assert_eq!(sheet["minimum_w"].as_f64(), Some(expected));

    let devices = sheet["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), classified.len());
    assert!(!devices.is_empty(), "the reference household has a wallbox");
    for device in devices {
        assert!(
            ["a", "b", "c", "d"].contains(&device["letter"].as_str().unwrap_or_default()),
            "{device}"
        );
        assert!(device["power_w"].as_f64().unwrap_or_default() > 4_200.0);
        assert!(device["minimum_direct_w"].as_f64().unwrap_or_default() >= 4_200.0);
    }
}

#[tokio::test]
async fn a_box_with_nothing_to_pair_says_so_in_the_trust_check() {
    let (address, _) = a_box().await;
    let trust: serde_json::Value = reqwest::Client::new()
        .get(format!("http://{address}/v1/trust"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer")
        .json()
        .await
        .expect("a document");
    assert_eq!(trust["identity"], "none");
    assert_eq!(trust["control_box"]["state"], "none");
    assert_eq!(trust["ski"], SKI);
}

/// Discovery is the box opening sockets on request, so what it may be asked to
/// open is narrow: literal addresses on a private network.
#[tokio::test]
async fn discovery_looks_only_where_a_household_network_is() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    for hosts in [
        serde_json::json!(["8.8.8.8"]),
        serde_json::json!(["inverter.example"]),
        serde_json::json!(["192.168.1.40:22"]),
        serde_json::json!((0..17).map(|i| format!("10.0.0.{i}")).collect::<Vec<_>>()),
    ] {
        let status = client
            .post(format!("{base}/v1/discover"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "hosts": hosts }))
            .send()
            .await
            .expect("the box to answer")
            .status();
        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{hosts}");
    }

    let any_port = client
        .post(format!("{base}/v1/discover"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "hosts": ["192.168.1.40"], "port": 22 }))
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(
        any_port.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "a port of the caller's choosing is a port scanner"
    );

    // 127.0.0.2 is loopback, and nothing listens on its 502.
    let found: serde_json::Value = client
        .post(format!("{base}/v1/discover"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "hosts": ["127.0.0.2"] }))
        .send()
        .await
        .expect("the box to answer")
        .json()
        .await
        .expect("a document");
    assert_eq!(found["modbus"][0]["host"], "127.0.0.2");
    assert_eq!(found["modbus"][0]["state"], "silent");
    assert!(found["ship"].is_array());
}

/// What makes a printed sheet safe to lose: the household issues itself a new
/// token, and everything made with the old one is dead on the next request.
#[tokio::test]
async fn replacing_the_token_ends_every_session_and_every_sheet() {
    let (address, access) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let cookie = pair(&cookie_of(&sign_in(&client, &base, TOKEN).await));
    let manager = access
        .connect_manager("aggregator-nord")
        .await
        .expect("a manager can be connected");
    async fn status_of(request: reqwest::RequestBuilder) -> reqwest::StatusCode {
        request.send().await.expect("the box to answer").status()
    }

    // Not on the cookie, and not by a manager: the answer is a credential.
    assert_eq!(
        status_of(
            client
                .post(format!("{base}/v1/token"))
                .header(reqwest::header::COOKIE, &cookie)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
        )
        .await,
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(
            client
                .post(format!("{base}/v1/token"))
                .bearer_auth(&manager)
        )
        .await,
        reqwest::StatusCode::FORBIDDEN
    );

    let answer = client
        .post(format!("{base}/v1/token"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer");
    assert_eq!(answer.status(), reqwest::StatusCode::OK);
    assert_eq!(answer.headers()[reqwest::header::CACHE_CONTROL], "no-store");
    let new: serde_json::Value = answer.json().await.expect("a document");
    let new = new["token"].as_str().expect("a token").to_owned();
    assert_ne!(new, TOKEN);
    assert_eq!(new.len(), 64);

    let status = format!("{base}/v1/status");
    assert_eq!(
        status_of(client.get(&status).bearer_auth(TOKEN)).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "the old token still opens the box"
    );
    assert_eq!(
        status_of(client.get(&status).header(reqwest::header::COOKIE, &cookie)).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "a session signed in with the old token survived"
    );
    assert_eq!(
        sign_in(&client, &base, TOKEN).await.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "the old sheet still signs in"
    );
    assert_eq!(
        status_of(client.get(&status).bearer_auth(&new)).await,
        reqwest::StatusCode::OK
    );
    assert_eq!(
        status_of(client.get(&status).bearer_auth(&manager)).await,
        reqwest::StatusCode::OK,
        "a manager never held the household's token and keeps its own"
    );
}

// ── Over HTTPS ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn over_https_the_cookie_is_secure_and_the_pinned_certificate_is_the_boxs() {
    let tls = a_box_over_tls().await;
    let base = format!("https://localhost:{}", tls.port);
    let client = pinned(&tls.fingerprint);

    let response = sign_in(&client, &base, TOKEN).await;
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    let cookie = cookie_of(&response);
    assert!(cookie.contains("; Secure"), "{cookie}");

    let status = client
        .get(format!("{base}/v1/status"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("the box to answer")
        .status();
    assert_eq!(status, reqwest::StatusCode::OK);

    // Somebody else's certificate on the same port is not this box.
    let other = pinned(&"00".repeat(32));
    assert!(
        other.get(format!("{base}/v1/status")).send().await.is_err(),
        "a client pinned to another certificate accepted this one"
    );
    // And the port does not also speak clear.
    assert!(
        reqwest::Client::new()
            .get(format!("http://localhost:{}/v1/status", tls.port))
            .bearer_auth(TOKEN)
            .send()
            .await
            .is_err(),
        "the HTTPS port answered a request in clear"
    );
}

fn battery(site: &Site) -> AssetId {
    site.assets
        .iter()
        .find_map(|a| match a {
            Asset::Battery(b) => Some(b.meta.id.clone()),
            _ => None,
        })
        .expect("the reference household has a battery")
}

/// S2 Connect: S2 must not travel unencrypted. So the manager dials `wss://`,
/// pins the certificate whose fingerprint the box prints, and the whole
/// handshake — through to an active control type — runs under it, with the
/// engine's validator reading every message the box sends.
#[tokio::test]
async fn a_manager_drives_the_box_over_wss_with_the_certificate_pinned() {
    let tls = a_box_over_tls().await;
    let asset = battery(&tls.site);
    let url = format!("wss://localhost:{}/s2/{asset}", tls.port);
    let policy = TlsPolicy::Pinned(Fingerprint::parse(&tls.fingerprint).expect("a fingerprint"));
    let options = WebSocketOptions::default().bearer(TOKEN).policy(&policy);
    let socket = WebSocket::open(&url, &options)
        .await
        .expect("the box to accept a manager over TLS");
    let mut session = CemSession::new(CemConfig::default());
    session.open(Timestamp::now());
    let mut driver = Driver::new(socket);
    let mut seen: Vec<CemEvent> = Vec::new();

    let mut selected = false;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            while let Some(event) = session.poll_event() {
                if matches!(event, CemEvent::ResourceDescribed(_)) && !selected {
                    session
                        .select_control_type(ControlType::FillRateBasedControl, Timestamp::now())
                        .expect("the selection to go out");
                    selected = true;
                }
                let ready = matches!(event, CemEvent::Ready { .. });
                seen.push(event);
                if ready {
                    return;
                }
            }
            driver
                .step(&mut session)
                .await
                .expect("the box to stay on the wire");
        }
    })
    .await
    .expect("the control type to become active over wss");

    for event in &seen {
        let report = match event {
            CemEvent::Refused { report, .. } | CemEvent::Warnings { report, .. } => report,
            _ => continue,
        };
        assert!(
            report.violations().is_empty(),
            "the manager objects to what the box sent: {:?}",
            report.violations().first().map(|v| &v.message)
        );
    }

    // The wrong certificate, and no certificate checking at all in clear.
    let wrong = TlsPolicy::Pinned(Fingerprint::parse(&"00".repeat(32)).expect("a fingerprint"));
    assert!(
        WebSocket::open(
            &url,
            &WebSocketOptions::default().bearer(TOKEN).policy(&wrong)
        )
        .await
        .is_err(),
        "a manager pinned to another certificate connected"
    );
    let clear = format!("ws://localhost:{}/s2/{asset}", tls.port);
    assert!(
        WebSocket::open(
            &clear,
            &WebSocketOptions::default()
                .bearer(TOKEN)
                .allow_insecure_bearer()
        )
        .await
        .is_err(),
        "the box spoke S2 in clear on its HTTPS port"
    );
}

// ── The demo ────────────────────────────────────────────────────────────────

/// The demo asks for no credential, so it is this machine's and nobody else's.
#[test]
fn the_demo_refuses_an_address_off_this_machine() {
    for off in ["0.0.0.0:8080", "192.168.1.5:8080", "[::]:8080"] {
        let error = hemsd::demo::loopback_only(off.parse().expect("an address"))
            .expect_err("it listened off this machine");
        assert!(
            error.to_string().contains("only on this machine"),
            "{error}"
        );
    }
    for on in ["127.0.0.1:8080", "[::1]:8080"] {
        hemsd::demo::loopback_only(on.parse().expect("an address")).expect("loopback is fine");
    }
}

/// The command refuses before it simulates anything, and says why.
#[test]
fn the_demo_command_refuses_before_it_simulates_a_day() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hemsd"))
        .args(["demo", "--listen", "0.0.0.0:0"])
        .output()
        .expect("the binary to run");
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("only on this machine"), "{said}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("simulating"),
        "it simulated a day before refusing"
    );
}

/// Every refusal under `/v1/` is one RFC 9457 problem document — the handler's
/// own sentence, an extractor's rejection, the access layer's 401 and a path
/// that does not exist alike.
#[tokio::test]
async fn every_refusal_is_a_problem_document() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let auth = |r: reqwest::RequestBuilder| r.bearer_auth(TOKEN);
    let cases: Vec<(&str, reqwest::RequestBuilder, u16)> = vec![
        ("no credential", client.get(format!("{base}/v1/home")), 401),
        (
            "a path that does not exist",
            auth(client.get(format!("{base}/v1/nothing-here"))),
            404,
        ),
        (
            "a series nobody records",
            auth(client.get(format!("{base}/v1/series/no-such-point"))),
            404,
        ),
        (
            "a body that is not JSON",
            auth(
                client
                    .put(format!("{base}/v1/goals/car"))
                    .header("content-type", "application/json")
                    .body("{not json"),
            ),
            400,
        ),
        (
            "forgetting a control box nobody trusts",
            auth(client.delete(format!("{base}/v1/pairing/{SKI}"))),
            404,
        ),
        (
            "withdrawing overrides when none is in force",
            auth(client.delete(format!("{base}/v1/overrides"))),
            404,
        ),
    ];
    for (what, request, status) in cases {
        let response = request.send().await.expect("an answer");
        assert_eq!(response.status().as_u16(), status, "{what}");
        let media = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert_eq!(media, "application/problem+json", "{what}");
        let problem: serde_json::Value = response.json().await.expect("a JSON body");
        assert_eq!(problem["status"], status, "{what}");
        assert!(
            problem["type"]
                .as_str()
                .is_some_and(|t| t.starts_with("https://hupe1980.github.io/hems/docs/api/#")),
            "{what}: {problem}"
        );
        assert!(
            problem["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "{what}: a sentence a person can act on"
        );
    }
}

/// The contract is served by the box it describes, without a credential.
#[tokio::test]
async fn the_box_serves_its_own_contract() {
    let (address, _) = a_box().await;
    let base = format!("http://{address}");
    let document: serde_json::Value = reqwest::get(format!("{base}/openapi.json"))
        .await
        .expect("an answer")
        .json()
        .await
        .expect("JSON");
    assert_eq!(document["openapi"], "3.1.0");
    assert!(document["components"]["schemas"]["Problem"].is_object());
}

/// A quote is two solves, so the box answers at most one a second.
#[tokio::test]
async fn quotes_come_one_at_a_time() {
    let (address, _) = a_box().await;
    let client = reqwest::Client::new();
    let ask = || {
        client
            .post(format!("http://{address}/v1/goals/wallbox/quote"))
            .bearer_auth(TOKEN)
            .json(&serde_json::json!({ "target_soc": 0.8, "by_minute": 420 }))
            .send()
    };
    // Nothing planned on this box: the first quote takes the slot and says so…
    assert_eq!(ask().await.unwrap().status().as_u16(), 503);
    // …and the next, within the second, is turned away before any solve.
    let again = ask().await.unwrap();
    assert_eq!(again.status().as_u16(), 429);
    let problem: serde_json::Value = again.json().await.unwrap();
    assert!(problem["type"].as_str().unwrap().ends_with("#too-many"));
}
