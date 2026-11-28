//! A box arriving, being adopted, asking what to run, and being offered an
//! update it can check for itself.

use std::collections::BTreeMap;
use std::sync::Arc;

use ed25519_dalek::{Signer, SigningKey};
use ems_service::update::{Manifest, Release, SignedConfig};
use fleetd::api::{Fleet, router};
use fleetd::config::SiteEntry;
use fleetd::registry::Registry;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const ARTEFACT: &[u8] = b"a gateway image";

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

fn release() -> Release {
    let manifest = Manifest {
        component: "hemsd".into(),
        version: "0.2.0".into(),
        url: "https://updates.example/hemsd-0.2.0".into(),
        sha256: hex::encode(Sha256::digest(ARTEFACT)),
        size_bytes: ARTEFACT.len() as u64,
        published_at: time::macros::datetime!(2026-06-21 12:00:00 UTC),
    };
    let signature = signing_key().sign(&manifest.signing_payload().unwrap());
    Release {
        manifest,
        signature: hex::encode(signature.to_bytes()),
    }
}

/// The configuration document for a site, signed the way an operator signs one.
///
/// `fleetd` never holds the signing key: it is applied here, off the fleet
/// server, and `fleetd` carries only the result. A `fleetd` an attacker owns can
/// therefore serve no configuration any box will accept.
fn signed_config(site: &str, version: &str, config: &str) -> SignedConfig {
    let mut c = SignedConfig {
        site: site.into(),
        version: version.into(),
        config: config.into(),
        signature: String::new(),
    };
    c.signature = hex::encode(signing_key().sign(&c.signing_payload().unwrap()).to_bytes());
    c
}

fn sites() -> BTreeMap<String, SiteEntry> {
    let c = signed_config("site-1", "7", "listen = \"0.0.0.0:8080\"\n");
    [(
        "site-1".to_owned(),
        SiteEntry {
            enrolment_secret: ems_service::Secret::literal("installer-secret"),
            config: c.config,
            config_version: c.version,
            config_signature: c.signature,
        },
    )]
    .into_iter()
    .collect()
}

async fn start() -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    start_with(sites()).await
}

async fn start_with(
    sites: BTreeMap<String, SiteEntry>,
) -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    let fixture = ems_service::testdb::Postgres::start(fleetd::store::MIGRATIONS).await;
    let db = fixture.db.clone();
    // Leaked on purpose: the pool outlives the server task, and a test process
    // ends when the test does.
    std::mem::forget(fixture);
    start_on(sites, db).await
}

/// A daemon whose durable half is `db`.
///
/// Two of these in a row over one database is a restart, which is the only way
/// to test the property the store exists for.
async fn start_on(
    sites: BTreeMap<String, SiteEntry>,
    db: ems_service::Db,
) -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = listener.local_addr().unwrap();
    drop(listener);
    let settings = ems_service::Settings {
        listen: bound,
        shutdown_grace_s: 2,
        ..ems_service::Settings::default()
    };
    let store = fleetd::store::Store::new(db);
    let registry = Arc::new(RwLock::new(Registry::restore(
        sites,
        store.enrolments().await.unwrap(),
        store.reports().await.unwrap(),
    )));
    let releases = [("hemsd".to_owned(), release())].into_iter().collect();
    let (signal, trigger) = ems_service::Shutdown::channel();
    let server = ems_service::Server::new(
        ems_service::identity!(),
        settings,
        ems_service::Health::new(),
        router(Fleet::new(
            registry,
            releases,
            ems_service::Credentials::default()
                .with_operator(OPERATOR)
                .with_operator_of(
                    ems_service::auth::SiteScope::One("sim-1".into()),
                    "canary-reader",
                )
                .with_operator_of(
                    ems_service::auth::SiteScope::One("other".into()),
                    "other-operator",
                ),
            store,
        )),
    );
    tokio::spawn(async move { server.run_until(signal).await.unwrap() });
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(bound).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    (bound, trigger)
}

async fn request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let head = match body {
        Some(b) => format!(
            "{method} {path} HTTP/1.1\r\nHost: h\r\n{auth}Content-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{b}",
            b.len()
        ),
        None => format!("{method} {path} HTTP/1.1\r\nHost: h\r\n{auth}Connection: close\r\n\r\n"),
    };
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        status,
        raw.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned(),
    )
}

#[tokio::test]
async fn a_box_is_adopted_told_what_to_run_and_reports_back() {
    let (address, trigger) = start().await;

    let (status, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    assert_eq!(status, 200);
    let enrolled: serde_json::Value = serde_json::from_str(&body).unwrap();
    let token = enrolled["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64, "256 bits of hexadecimal");

    let (status, body) = request(address, "GET", "/v1/config", Some(&token), None).await;
    assert_eq!(status, 200);
    let config: SignedConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(config.version, "7");
    // The box checks it against the key it was **built** with, so what the fleet
    // served is checkable without trusting the fleet.
    let public = hex::encode(signing_key().verifying_key().to_bytes());
    assert!(
        config
            .verify_for("site-1", &public)
            .unwrap()
            .contains("listen"),
        "the document a box would run"
    );

    // Before the box reports, the fleet does not claim it has converged.
    let (_, body) = request(address, "GET", "/v1/fleet", Some(OPERATOR), None).await;
    let states: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(states[0]["converged"], false);

    let (status, _) = request(
        address,
        "POST",
        "/v1/config/running",
        Some(&token),
        Some(r#"{"config_version":"7"}"#),
    )
    .await;
    assert_eq!(status, 204);
    let (_, body) = request(address, "GET", "/v1/fleet", Some(OPERATOR), None).await;
    let states: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(states[0]["converged"], true);
    trigger.trigger();
}

/// The credential an operator reads the roster with.
const OPERATOR: &str = "tok-operator";

#[tokio::test]
async fn the_roster_is_not_public() {
    // It lists every household this fleet has adopted, the version each is on
    // and when it was last heard from — which is to say which households exist,
    // which are running an old build and which are unreachable right now. A
    // box's own enrolment credential does not read it either: that one says
    // "I am this household", and this is about every household.
    let (address, trigger) = start().await;

    let (status, _) = request(address, "GET", "/v1/fleet", None, None).await;
    assert_eq!(status, 401, "no credential, no roster");

    let (status, _) = request(address, "GET", "/v1/fleet", Some("tok-nobody"), None).await;
    assert_eq!(status, 401, "and not somebody else's either");

    let (status, _) = request(address, "GET", "/v1/fleet", Some(OPERATOR), None).await;
    assert_eq!(status, 200, "an operator's credential does");
    trigger.trigger();
}

#[tokio::test]
async fn a_configuration_from_a_fleet_server_that_lies_is_not_accepted() {
    // The property the signature buys, and the reason it is not enough for this
    // endpoint to be authenticated: a `fleetd` an attacker owns is still a
    // `fleetd` no box will take a configuration from.
    let (address, trigger) = start_with(sites_with_a_tampered_document()).await;
    let (_, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    let token = serde_json::from_str::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = request(address, "GET", "/v1/config", Some(&token), None).await;
    assert_eq!(status, 200, "the fleet serves it happily");
    let config: SignedConfig = serde_json::from_str(&body).unwrap();
    let public = hex::encode(signing_key().verifying_key().to_bytes());
    assert!(
        config.verify_for("site-1", &public).is_err(),
        "and the box refuses it"
    );
    trigger.trigger();
}

/// A site whose document has been edited after it was signed.
fn sites_with_a_tampered_document() -> BTreeMap<String, SiteEntry> {
    let mut sites = sites();
    let entry = sites.get_mut("site-1").unwrap();
    entry.config = "listen = \"0.0.0.0:1\"\n".into();
    sites
}

#[tokio::test]
async fn an_unsigned_configuration_is_not_published_at_all() {
    // An operator who has not signed a document has not published one. Serving
    // it unsigned would need the box to have a "trust it anyway" path, and that
    // path is the one an attacker aims at.
    let mut sites = sites();
    sites.get_mut("site-1").unwrap().config_signature = String::new();
    let (address, trigger) = start_with(sites).await;
    let (_, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    let token = serde_json::from_str::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = request(address, "GET", "/v1/config", Some(&token), None).await;
    assert_eq!(status, 503);
    trigger.trigger();
}

#[tokio::test]
async fn every_bad_enrolment_gets_the_same_answer() {
    // An endpoint that says "no such site" for one input and "wrong secret" for
    // another will enumerate the fleet for anybody who asks patiently.
    let (address, trigger) = start().await;
    for body in [
        r#"{"site":"nobody","secret":"installer-secret"}"#,
        r#"{"site":"site-1","secret":"wrong"}"#,
    ] {
        let (status, answer) = request(address, "POST", "/v1/enrol", None, Some(body)).await;
        assert_eq!(status, 401, "{body}");
        assert!(answer.trim().is_empty(), "and it says nothing: {answer}");
    }
    trigger.trigger();
}

#[tokio::test]
async fn nothing_is_served_without_a_credential() {
    let (address, trigger) = start().await;
    for path in ["/v1/config", "/v1/releases/hemsd"] {
        assert_eq!(
            request(address, "GET", path, None, None).await.0,
            401,
            "{path}"
        );
        assert_eq!(
            request(address, "GET", path, Some("invented"), None)
                .await
                .0,
            401,
            "{path}"
        );
    }
    trigger.trigger();
}

#[tokio::test]
async fn the_offered_release_verifies_against_the_key_the_box_was_built_with() {
    // The whole point of signing: the box trusts a key, not a server. A
    // compromised `fleetd` can serve a manifest, and no box will accept it.
    let (address, trigger) = start().await;
    let (_, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    let token = serde_json::from_str::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = request(address, "GET", "/v1/releases/hemsd", Some(&token), None).await;
    assert_eq!(status, 200);
    let offered: Release = serde_json::from_str(&body).unwrap();

    let trusted = hex::encode(signing_key().verifying_key().to_bytes());
    let manifest = offered.verify(&trusted).expect("signed by us");
    assert_eq!(manifest.version, "0.2.0");
    offered
        .check_artefact(ARTEFACT)
        .expect("the right artefact");
    offered.check_newer("0.1.0").expect("newer than what runs");

    // …and a box built with somebody else's key refuses it, which is the same
    // sentence read from the other side.
    let stranger = hex::encode(
        SigningKey::from_bytes(&[1u8; 32])
            .verifying_key()
            .to_bytes(),
    );
    assert!(offered.verify(&stranger).is_err());

    // A release for a component this fleet does not carry is a 404, not an
    // empty manifest a box might act on.
    assert_eq!(
        request(address, "GET", "/v1/releases/nothing", Some(&token), None)
            .await
            .0,
        404
    );
    trigger.trigger();
}

#[tokio::test]
async fn a_restart_does_not_orphan_the_fleet() {
    // The registry used to be entirely in memory, seeded from TOML. Restarting
    // the daemon threw away every credential it had ever issued — and the box
    // holding the other copy could not enrol again, because its single-use
    // secret was spent. This is that property, and it is the reason `fleetd`
    // has a database at all.
    let fixture = ems_service::testdb::Postgres::start(fleetd::store::MIGRATIONS).await;

    let (address, trigger) = start_on(sites(), fixture.db.clone()).await;
    let (status, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let token = serde_json::from_str::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .expect("a minted credential")
        .to_owned();

    let (status, _) = request(
        address,
        "POST",
        "/v1/config/running",
        Some(&token),
        Some(r#"{"config_version":"7"}"#),
    )
    .await;
    assert_eq!(status, 204);
    trigger.trigger();

    // …and now the daemon comes back, against the same database.
    let (address, trigger) = start_on(sites(), fixture.db.clone()).await;

    let (status, body) = request(address, "GET", "/v1/config", Some(&token), None).await;
    assert_eq!(
        status, 200,
        "the box's credential still works after a restart: {body}"
    );

    let (status, _) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    assert_eq!(
        status, 401,
        "and the single-use secret is still spent — otherwise every enrolment \
         secret an installer wrote down becomes live again on every deploy"
    );

    let (status, body) = request(address, "GET", "/v1/fleet", Some(OPERATOR), None).await;
    assert_eq!(status, 200);
    let roster: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        roster[0]["running_version"], "7",
        "and the rollout answer survived too: a fleet that forgot it would \
         report a completed rollout as untouched"
    );
    assert_eq!(roster[0]["converged"], true);

    trigger.trigger();
}

#[tokio::test]
async fn desk_distinguishes_stalled_meter_and_rejects_foreign_access_and_replays() {
    let fixture = ems_service::testdb::Postgres::start(fleetd::store::MIGRATIONS).await;
    let (address, trigger) = start_on(sites(), fixture.db.clone()).await;
    let (_, body) = request(
        address,
        "POST",
        "/v1/enrol",
        None,
        Some(r#"{"site":"site-1","secret":"installer-secret"}"#),
    )
    .await;
    let enrolment: serde_json::Value = serde_json::from_str(&body).unwrap();
    let token = enrolment["token"].as_str().unwrap();
    let read = |token| request(address, "GET", "/v1/desk?site=site-1", Some(token), None);
    let sites = |body: String| serde_json::from_str::<serde_json::Value>(&body).unwrap();
    assert_eq!(
        sites(read(OPERATOR).await.1)[0]["attention"],
        "Awaiting first check-in"
    );
    assert_eq!(read(token).await.0, 401);
    assert_eq!(sites(read("other-operator").await.1), serde_json::json!([]));
    let now = time::OffsetDateTime::now_utc().unix_timestamp() - 5;
    for offset in (-36..=0).chain([-1, 0]) {
        let payload = serde_json::json!({"sent_at":now+offset,"feeder_at":now-180}).to_string();
        assert_eq!(
            request(address, "POST", "/v1/checkins", Some(token), Some(&payload))
                .await
                .0,
            204
        );
    }
    for payload in [
        serde_json::json!({"sent_at":now-100}),
        serde_json::json!({"sent_at":now,"feeder_at":now+1}),
    ] {
        assert_eq!(
            request(
                address,
                "POST",
                "/v1/checkins",
                Some(token),
                Some(&payload.to_string())
            )
            .await
            .0,
            422
        );
    }
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/checkins",
            Some(OPERATOR),
            Some(&serde_json::json!({"sent_at":now}).to_string())
        )
        .await
        .0,
        401
    );
    let result = sites(read(OPERATOR).await.1);
    let row = &result[0];
    assert_eq!(row["attention"], "Inspect feeder");
    assert_eq!(row["gateway"]["state"], "current");
    assert_eq!(row["feeder"]["state"], "stale");
    assert_eq!(row["report"]["state"], "unknown");
    assert_eq!(row["history"].as_array().unwrap().len(), 32);
    assert_eq!(row["history"][0]["sent_at"], now);
    assert_eq!(row["history"][31]["sent_at"], now - 31);
    let client = fixture.db.get().await.unwrap();
    client
        .execute(
            "UPDATE gateway_checkin SET received_at=$1 WHERE site='site-1' AND sent_at=$2",
            &[&(now - 95), &now],
        )
        .await
        .unwrap();
    let duplicate = serde_json::json!({"sent_at":now,"feeder_at":now}).to_string();
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/checkins",
            Some(token),
            Some(&duplicate)
        )
        .await
        .0,
        204
    );
    let result = sites(read(OPERATOR).await.1);
    assert_eq!(result[0]["attention"], "Gateway silent");
    assert_eq!(result[0]["history"][0]["received_at"], now - 95);
    let recovered =
        serde_json::json!({"sent_at":now+1,"feeder_at":now,"report_at":now-37*3600}).to_string();
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/checkins",
            Some(token),
            Some(&recovered)
        )
        .await
        .0,
        204
    );
    let result = sites(read(OPERATOR).await.1);
    assert_eq!(result[0]["attention"], "Inspect closed report");
    assert_eq!(result[0]["gateway"]["state"], "current");
    assert_eq!(result[0]["report"]["state"], "stale");
    assert_eq!(result[0]["history"][1]["feeder_at"], now - 180);
    client
        .execute(
            "INSERT INTO enrolment (site, token, enrolled_at)
        SELECT 'sim-' || n, 'test-' || n, now() FROM generate_series(2,10000) n",
            &[],
        )
        .await
        .unwrap();
    let summary = sites(
        request(address, "GET", "/v1/desk", Some(OPERATOR), None)
            .await
            .1,
    );
    assert_eq!(summary["total"], 10_000);
    assert_eq!(summary["rows"].as_array().unwrap().len(), 15);
    let search = sites(
        request(
            address,
            "GET",
            "/v1/desk?q=sim-999&filter=attention&page=100",
            Some(OPERATOR),
            None,
        )
        .await
        .1,
    );
    assert_eq!(search["matched"], 11);
    assert_eq!(search["page"], 0);
    let second = sites(
        request(address, "GET", "/v1/desk?page=1", Some(OPERATOR), None)
            .await
            .1,
    );
    assert_eq!(second["page"], 1);
    assert_ne!(second["rows"][0]["site"], summary["rows"][0]["site"]);
    let scoped = sites(
        request(
            address,
            "GET",
            "/v1/desk?q=site-1",
            Some("other-operator"),
            None,
        )
        .await
        .1,
    );
    assert_eq!(scoped["matched"], 0);
    assert!(
        summary["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["history"].as_array().unwrap().is_empty())
    );
    trigger.trigger();
}

#[tokio::test]
async fn canary_freezes_membership_and_requires_fresh_scoped_boot_evidence() {
    let fixture = ems_service::testdb::Postgres::start(fleetd::store::MIGRATIONS).await;
    let (addr, stop) = start_on(sites(), fixture.db.clone()).await;
    let store = fleetd::store::Store::new(fixture.db.clone());
    let db = fixture.db.get().await.unwrap();
    db.execute(
        "INSERT INTO enrolment SELECT 'sim-'||n,'token-'||n,now() FROM generate_series(1,100) n",
        &[],
    )
    .await
    .unwrap();
    let digest = release().manifest.sha256;
    let run = store.freeze_rehearsal("sim-1", &digest).await.unwrap();
    db.execute(
        "INSERT INTO enrolment VALUES ('late','late-token',now())",
        &[],
    )
    .await
    .unwrap();
    let endpoint = format!("/v1/rehearsals/{run}/observations");
    let read = || request(addr, "GET", "/v1/desk/canary", Some(OPERATOR), None);
    let verified=serde_json::json!({"run":run,"phase":"verified","candidate":digest,"initial_boot":"old","verdict":"verified"}).to_string();
    let ready=serde_json::json!({"run":run,"phase":"running","sent_at":time::OffsetDateTime::now_utc().unix_timestamp(),"digest":digest,"boot":"new","ready":true,"rollback":false}).to_string();
    assert_eq!(
        request(addr, "GET", "/v1/desk/canary", Some("token-1"), None)
            .await
            .0,
        401
    );
    assert_eq!(
        request(addr, "GET", "/v1/desk/canary", Some("other-operator"), None)
            .await
            .1,
        "null"
    );
    assert_eq!(
        request(addr, "POST", &endpoint, Some("token-2"), Some(&verified))
            .await
            .0,
        404
    );
    assert_eq!(
        request(addr, "POST", &endpoint, Some("token-1"), Some(&ready))
            .await
            .0,
        422
    );
    let previous = verified.replace(&run, "older-run");
    assert_eq!(
        request(addr, "POST", &endpoint, Some("token-1"), Some(&previous))
            .await
            .0,
        422
    );
    let scoped = format!("/v1/desk/canary?run={run}");
    assert_eq!(
        request(addr, "GET", &scoped, Some("other-operator"), None)
            .await
            .1,
        "null"
    );
    for payload in [&verified, &verified, &ready, &ready] {
        assert_eq!(
            request(addr, "POST", &endpoint, Some("token-1"), Some(payload))
                .await
                .0,
            204
        );
    }
    let review: serde_json::Value = serde_json::from_str(&read().await.1).unwrap();
    assert_eq!(review["cohort"], 100);
    let scoped_review: serde_json::Value = serde_json::from_str(
        &request(addr, "GET", &scoped, Some("canary-reader"), None)
            .await
            .1,
    )
    .unwrap();
    assert_eq!(scoped_review["cohort"], 1);
    let cohort_page = format!("/v1/desk?filter=cohort&run={run}");
    let scoped_page: serde_json::Value = serde_json::from_str(
        &request(addr, "GET", &cohort_page, Some("canary-reader"), None)
            .await
            .1,
    )
    .unwrap();
    assert_eq!(scoped_page["matched"], 1);
    assert_eq!(scoped_page["rows"][0]["site"], "sim-1");
    assert_eq!(review["decision"], "Qualified for next simulated site");
    db.execute(
        "UPDATE rehearsal SET observed_at=now()-interval '121 seconds' WHERE id=$1",
        &[&run],
    )
    .await
    .unwrap();
    assert_eq!(
        request(addr, "POST", &endpoint, Some("token-1"), Some(&ready))
            .await
            .0,
        204
    );
    assert!(read().await.1.contains("stale readiness"));
    let conflict = ready.replace("\"ready\":true", "\"ready\":false");
    assert_eq!(
        request(addr, "POST", &endpoint, Some("token-1"), Some(&conflict))
            .await
            .0,
        409
    );
    assert!(read().await.1.contains("conflicting observations"));
    stop.trigger();
}
