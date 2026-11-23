//! Who may read a household's record, and who may write it.
//!
//! These routes serve the two documents this daemon exists for: a network
//! operator's Nachweis `[A1 7.2]`, and the household's Data Act Article 4
//! export — everything the product generated, which is when the shower ran and
//! which fortnight nobody was in. Authorisation is therefore part of what the
//! service *is*, and every rule below is checked over a real socket.

mod common;

use std::collections::BTreeMap;

use ems_core::prelude::{GuardRule, Power};
use ems_grid::evidence::ControlEvent;
use ems_grid::mispel::QuarterHour;
use ems_grid::para14a::ControlMode;
use ems_service::{Credentials, Secret};
use histd::api::{History, router};
use time::macros::datetime;

const NOW: time::OffsetDateTime = datetime!(2026-01-15 17:00:00 UTC);

const HAUS_1: &str = "tok-haus-1";
const HAUS_2: &str = "tok-haus-2";
const NETZ: &str = "tok-netzbetreiber";

fn credentials() -> Credentials {
    let sites: BTreeMap<String, Secret> = [
        ("haus-1".to_owned(), Secret::literal(HAUS_1)),
        ("haus-2".to_owned(), Secret::literal(HAUS_2)),
    ]
    .into_iter()
    .collect();
    Credentials::resolve(
        &sites,
        &std::collections::BTreeMap::new(),
        &[ems_service::OperatorCredential {
            token: Secret::literal(NETZ),
            tenant: "*".into(),
        }],
    )
    .unwrap()
}

fn event() -> ControlEvent {
    ControlEvent::received(
        GuardRule::Lpc,
        ControlMode::Ems,
        Power::from_kw(4.2),
        Power::from_kw(10.5),
        NOW,
    )
}

async fn start() -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    start_with_register_meter(false).await
}

async fn start_with_register_meter(
    commissioned: bool,
) -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = listener.local_addr().unwrap();
    drop(listener);

    // A record already exists for `haus-1`, so a refusal below is a refusal
    // rather than an empty answer that happens to look like one.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = histd::Store::new(fixture.db.clone());
    let store = if commissioned {
        store
            .with_register_meters(
                &common::database(&fixture),
                BTreeMap::from([(
                    "haus-1".into(),
                    histd::config::RegisterMeter {
                        malo: "12345678905".parse().unwrap(),
                        operator: "9900000000003".parse().unwrap(),
                    },
                )]),
            )
            .await
            .unwrap()
    } else {
        store
    };
    store
        .put_control_event("haus-1", &event())
        .await
        .expect("a record to be refused access to");
    store
        .put_quarter_hour(
            "haus-1",
            &QuarterHour::empty(ems_core::prelude::Slot::containing(NOW)),
            NOW,
        )
        .await
        .expect("a register");

    let settings = ems_service::Settings {
        listen: bound,
        shutdown_grace_s: 2,
        ..ems_service::Settings::default()
    };
    let (signal, trigger) = ems_service::Shutdown::channel();
    let server = ems_service::Server::new(
        ems_service::identity!(),
        settings,
        ems_service::Health::new(),
        router(History::new(store, credentials()).settling(
            BTreeMap::from([(
                "haus-1".to_owned(),
                histd::config::MispelSettings::Ausschliesslichkeit,
            )]),
            None,
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

/// One HTTP/1.1 request, so the test needs no client dependency.
async fn request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> u16 {
    exchange(address, method, path, token, body).await.0
}

async fn exchange(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let auth = token.map_or_else(String::new, |t| format!("Authorization: Bearer {t}\r\n"));
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
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

#[tokio::test]
async fn nothing_is_served_without_a_credential() {
    let (address, trigger) = start().await;
    for path in [
        "/v1/sites/haus-1/export",
        "/v1/sites/haus-1/nachweis",
        "/v1/sites/haus-1/quarter-hours",
    ] {
        assert_eq!(
            request(address, "GET", path, None, None).await,
            401,
            "{path} answered without a token"
        );
        assert_eq!(
            request(address, "GET", path, Some("tok-invented"), None).await,
            401,
            "{path} answered a token nobody issued"
        );
    }
    trigger.trigger();
}

#[tokio::test]
async fn one_households_credential_does_not_reach_anothers_record() {
    // `403` and not `404`: the caller is somebody, just not this somebody, and
    // an operator debugging a rollout has to be able to tell those apart.
    let (address, trigger) = start().await;
    for path in [
        "/v1/sites/haus-1/export",
        "/v1/sites/haus-1/nachweis",
        "/v1/sites/haus-1/quarter-hours",
    ] {
        assert_eq!(
            request(address, "GET", path, Some(HAUS_2), None).await,
            403,
            "{path} served haus-2 the record of haus-1"
        );
    }
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/sites/haus-1/events",
            Some(HAUS_2),
            Some(&serde_json::to_string(&event()).unwrap()),
        )
        .await,
        403,
        "haus-2 wrote evidence into haus-1's record"
    );
    trigger.trigger();
}

#[tokio::test]
async fn a_box_reads_and_writes_its_own_record() {
    let (address, trigger) = start().await;
    for path in [
        "/v1/sites/haus-1/export",
        "/v1/sites/haus-1/nachweis",
        "/v1/sites/haus-1/quarter-hours",
    ] {
        assert_eq!(
            request(address, "GET", path, Some(HAUS_1), None).await,
            200,
            "{path}"
        );
    }
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/sites/haus-1/events",
            Some(HAUS_1),
            Some(&serde_json::to_string(&event()).unwrap()),
        )
        .await,
        201
    );
    trigger.trigger();
}

#[tokio::test]
async fn register_revisions_retry_conflict_or_correct_over_http() {
    use ems_grid::mispel::RegisterAssertion;
    let (address, trigger) = start_with_register_meter(true).await;
    let path = "/v1/sites/haus-1/quarter-hours";
    let mut value = RegisterAssertion {
        registers: QuarterHour::empty(ems_core::prelude::Slot::containing(NOW)),
        recorded_at: NOW + time::Duration::seconds(1),
    };
    let post = async |body: String| request(address, "POST", path, Some(HAUS_1), Some(&body)).await;
    let batch = |assertions: &[RegisterAssertion]| serde_json::to_string(assertions).unwrap();

    assert_eq!(post(batch(&[value, value])).await, 204);
    assert_eq!(post(batch(&[value])).await, 204);
    let original = value;
    value.registers.grid_draw = rust_decimal::Decimal::ONE;
    assert_eq!(post(batch(&[value])).await, 409);
    assert_eq!(post(batch(&[original, value])).await, 409);
    assert_eq!(
        request(address, "POST", path, Some(HAUS_2), Some(&batch(&[value]))).await,
        403
    );
    value.recorded_at += time::Duration::microseconds(1);
    assert_eq!(post(batch(&[value])).await, 204);

    let mut malformed = serde_json::to_value([value]).unwrap();
    malformed[0]["registers"]["slot"] = serde_json::json!("2026-01-15T17:03:00Z");
    assert_eq!(post(malformed.to_string()).await, 422);
    value.recorded_at += time::Duration::nanoseconds(1);
    assert_eq!(post(batch(&[value])).await, 422);
    trigger.trigger();
}

#[tokio::test]
async fn the_first_receipt_supplies_a_replayable_cutoff_even_for_a_future_request() {
    let (address, trigger) = start().await;
    let url = "/v1/sites/haus-1/mispel?year=2026&month=11";
    let receipt = async |path: String| {
        let (status, body) = exchange(address, "GET", &path, Some(HAUS_1), None).await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()
    };
    let slot = ems_core::prelude::Slot::containing(datetime!(2026-11-03 14:00 UTC));
    let submit = async |value: ems_grid::mispel::RegisterAssertion| {
        let batch = serde_json::to_string(&[value]).unwrap();
        assert_eq!(
            request(
                address,
                "POST",
                "/v1/sites/haus-1/quarter-hours",
                Some(HAUS_1),
                Some(&batch)
            )
            .await,
            204
        );
    };
    let mut value = ems_grid::mispel::RegisterAssertion {
        registers: QuarterHour::empty(slot),
        recorded_at: slot.start(),
    };
    submit(value).await;
    let first = receipt(url.to_owned()).await;
    let future = receipt(format!("{url}&as_of=2099-01-01T00:00:00Z")).await;
    value.recorded_at += time::Duration::microseconds(1);
    value.registers.grid_draw = rust_decimal::Decimal::ONE;
    value.registers.device_consumption = rust_decimal::Decimal::ONE;
    submit(value).await;
    let now = receipt(url.to_owned()).await;
    assert_eq!(now["held"], false);
    assert_eq!(now["quarter_hours"]["present"], 1);
    for old in [first, future] {
        let cutoff = old["as_of"].as_str().unwrap();
        assert!(cutoff.ends_with('Z'));
        assert_eq!(old["quarter_hours"]["present"], 1);
        assert_eq!(old["held"], true);
        assert_eq!(receipt(format!("{url}&as_of={cutoff}")).await, old);
    }
    trigger.trigger();
}

#[tokio::test]
async fn an_operator_reads_the_nachweis_and_never_the_data_act_export() {
    // The distinction the two documents are for. `[A1 7.2]` is the record of
    // what the operator commanded and what the connection point drew, and it is
    // theirs to check. Article 4 of Regulation (EU) 2023/2854 is a right of the
    // **user**, and a fleet token is not a household.
    let (address, trigger) = start().await;
    assert_eq!(
        request(
            address,
            "GET",
            "/v1/sites/haus-1/nachweis",
            Some(NETZ),
            None
        )
        .await,
        200
    );
    assert_eq!(
        request(address, "GET", "/v1/sites/haus-1/export", Some(NETZ), None).await,
        403,
        "an operator was handed the household's whole consumption record"
    );
    trigger.trigger();
}

#[tokio::test]
async fn an_operator_may_not_write_the_record_it_is_judged_by() {
    // An operator that could write the evidence of its own control actions is
    // marking its own homework, and `[A1 7.2]` puts the record with the
    // household for exactly that reason.
    let (address, trigger) = start().await;
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/sites/haus-1/events",
            Some(NETZ),
            Some(&serde_json::to_string(&event()).unwrap()),
        )
        .await,
        403
    );
    assert_eq!(
        request(
            address,
            "POST",
            "/v1/sites/haus-1/quarter-hours",
            Some(NETZ),
            Some("[]"),
        )
        .await,
        403
    );
    trigger.trigger();
}
