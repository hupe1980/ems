//! Events out, against a receiver that is not ours.
//!
//! A test server on this machine plays Home Assistant: it records what it is
//! sent and answers what the test tells it to. Every signature is checked by an
//! implementation of the Standard Webhooks rule written here from the
//! specification — HMAC-SHA256 over `id.timestamp.body`, keyed with the bytes
//! behind `whsec_` — and not by `ems-events`' own `verify`, so a mistake in
//! the signer cannot be agreed with by its twin.

use std::sync::Arc;

use aws_lc_rs::hmac;
use base64::Engine as _;
use hemsd::runtime::push::{Event, Lang, Occurrence, message};
use hemsd::runtime::webhooks::{Asked, Webhooks};
use tokio::sync::Mutex;

#[derive(Default)]
struct Received {
    bodies: Vec<(axum::http::HeaderMap, Vec<u8>)>,
    /// Answer this many requests with 500 before answering 200.
    fail_first: usize,
}

async fn receiver() -> (String, Arc<Mutex<Received>>) {
    let seen = Arc::new(Mutex::new(Received::default()));
    let state = Arc::clone(&seen);
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let state = Arc::clone(&state);
                async move {
                    let mut s = state.lock().await;
                    s.bodies.push((headers, body.to_vec()));
                    if s.fail_first > 0 {
                        s.fail_first -= 1;
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        axum::http::StatusCode::OK
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}/hook"), seen)
}

/// The Standard Webhooks check, from the specification.
fn verifies(secret: &str, headers: &axum::http::HeaderMap, body: &[u8]) -> bool {
    let key = base64::engine::general_purpose::STANDARD
        .decode(secret.trim_start_matches("whsec_"))
        .expect("a base64 secret");
    let id = headers["webhook-id"].to_str().unwrap();
    let timestamp = headers["webhook-timestamp"].to_str().unwrap();
    let mut signed = format!("{id}.{timestamp}.").into_bytes();
    signed.extend_from_slice(body);
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &key), &signed);
    let expected = format!(
        "v1,{}",
        base64::engine::general_purpose::STANDARD.encode(tag.as_ref())
    );
    headers["webhook-signature"]
        .to_str()
        .unwrap()
        .split(' ')
        .any(|s| s == expected)
}

fn reduced_home() -> serde_json::Value {
    serde_json::json!({
        "reduction": { "ceiling_kw": 4.2, "minimum_kw": 10.5, "failsafe": false },
        "goals": {},
    })
}

#[tokio::test]
async fn a_reduction_reaches_a_receiver_signed_and_nothing_else_does() {
    let (url, seen) = receiver().await;
    let hooks = Webhooks::new("haus-1", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    let created = hooks
        .subscribe(
            Asked {
                name: "Home Assistant".into(),
                url,
                types: vec!["de.ems.grid.*".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .expect("subscribed");
    assert!(created.secret.starts_with("whsec_"));

    let reduction = Occurrence::from(Event::ReductionBegan);
    hooks.emit(&[(reduction, None)], &reduced_home(), now).await;
    hooks
        .emit(&[(Event::GoalTight.into(), None)], &reduced_home(), now)
        .await; // not asked for
    hooks
        .emit(
            &[(Event::ReductionEnded.into(), None)],
            &serde_json::json!({}),
            now,
        )
        .await;
    hooks.deliver_due(now).await;

    let received = seen.lock().await;
    let push = message(reduction, Lang::En, "https://hems.local");
    let (headers, body) = &received.bodies[0];
    let cloud: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(push["notification"]["data"]["event_id"], cloud["id"]);
    assert_eq!(
        headers["webhook-id"].to_str().unwrap(),
        reduction.id.to_string()
    );
    let types: Vec<String> = received
        .bodies
        .iter()
        .map(|(_, body)| {
            let event: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(event["specversion"], "1.0");
            event["type"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        types,
        [
            "de.ems.grid.lpc.limit.received",
            "de.ems.grid.lpc.limit.released"
        ],
        "the reduction's two events, and not the goal it did not ask for"
    );
    for (headers, body) in &received.bodies {
        assert!(
            verifies(&created.secret, headers, body),
            "a signature that does not verify"
        );
        assert_eq!(headers["content-type"], "application/cloudevents+json");
    }
    let first: serde_json::Value = serde_json::from_slice(&received.bodies[0].1).unwrap();
    assert_eq!(first["data"]["limit_w"], 4200.0);
    assert_eq!(first["data"]["minimum_w"], 10500.0);
}

#[tokio::test]
async fn a_receiver_that_fails_gets_the_same_event_again_later() {
    let (url, seen) = receiver().await;
    seen.lock().await.fail_first = 1;
    let hooks = Webhooks::new("haus-1", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    let created = hooks
        .subscribe(
            Asked {
                name: "Node-RED".into(),
                url,
                types: vec!["de.ems.grid.lpc.limit.received".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    hooks
        .emit(
            &[(Event::ReductionBegan.into(), None)],
            &reduced_home(),
            now,
        )
        .await;
    hooks.deliver_due(now).await;
    // Not yet due again: the first gap is thirty seconds.
    hooks.deliver_due(now + time::Duration::seconds(5)).await;
    assert_eq!(seen.lock().await.bodies.len(), 1);
    hooks.deliver_due(now + time::Duration::seconds(31)).await;

    let received = seen.lock().await;
    assert_eq!(received.bodies.len(), 2);
    let id = |i: usize| {
        received.bodies[i].0["webhook-id"]
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        id(0),
        id(1),
        "a retry carries the same id, so a receiver de-duplicates"
    );
    drop(received);

    let log = hooks
        .deliveries(now + time::Duration::seconds(31))
        .await
        .unwrap();
    let outcomes: Vec<&str> = log.iter().map(|d| d.outcome.as_str()).collect();
    assert_eq!(outcomes, ["delivered", "retrying"], "newest first");
    assert_eq!(hooks.list().await.unwrap()[0].owed, 0);
    assert!(hooks.revoke(&created.view.id).await.unwrap());
    assert!(!hooks.revoke(&created.view.id).await.unwrap());
}

#[tokio::test]
async fn a_test_ping_says_what_the_receiver_answered() {
    let (url, seen) = receiver().await;
    let hooks = Webhooks::new("haus-1", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    let created = hooks
        .subscribe(
            Asked {
                name: "the installer".into(),
                url,
                types: vec!["de.ems.test.ping".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    let answer = hooks
        .test(&created.view.id, now)
        .await
        .unwrap()
        .expect("known");
    assert!(answer.delivered, "{answer:?}");
    assert_eq!(answer.status, Some(200));
    let (headers, body) = &seen.lock().await.bodies[0];
    assert!(verifies(&created.secret, headers, body));
}

#[tokio::test]
async fn the_box_refuses_to_send_where_the_household_did_not_point_it() {
    let hooks = Webhooks::new("haus-1", None);
    let now = time::OffsetDateTime::now_utc();
    let to = |url: &str, types: &[&str]| Asked {
        name: "x".into(),
        url: url.into(),
        types: types.iter().map(|t| (*t).to_owned()).collect(),
        subjects: vec![],
        public: false,
    };
    for (what, asked) in [
        (
            "loopback",
            to("http://127.0.0.1:9/hook", &["de.ems.grid.*"]),
        ),
        (
            "link-local",
            to("http://169.254.10.1/hook", &["de.ems.grid.*"]),
        ),
        (
            "an unknown type",
            to(
                "http://192.168.1.20/hook",
                &[format!("{}weather.*", ems_events::PREFIX).as_str()],
            ),
        ),
        ("no type", to("http://192.168.1.20/hook", &[])),
    ] {
        assert!(hooks.subscribe(asked, now).await.is_err(), "{what}");
    }
    assert!(hooks.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_ninth_system_is_refused() {
    let hooks = Webhooks::new("haus-1", None);
    let now = time::OffsetDateTime::now_utc();
    for i in 0..8 {
        hooks
            .subscribe(
                Asked {
                    name: format!("system {i}"),
                    url: format!("http://192.168.1.{}/hook", 10 + i),
                    types: vec!["de.ems.*".into()],
                    subjects: vec![],
                    public: false,
                },
                now,
            )
            .await
            .expect("room");
    }
    let ninth = hooks
        .subscribe(
            Asked {
                name: "one too many".into(),
                url: "http://192.168.1.99/hook".into(),
                types: vec!["de.ems.*".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap_err();
    assert!(ninth.to_string().contains("revoke one"), "{ninth}");
}

/// A system revoked while a delivery to it is out stays revoked: the pass
/// records its attempt on the subscription as it is then, not as it was read.
#[tokio::test]
async fn a_revoke_during_a_slow_delivery_holds() {
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(|| async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            axum::http::StatusCode::OK
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let hooks = Webhooks::new("haus-1", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    let created = hooks
        .subscribe(
            Asked {
                name: "slow".into(),
                url,
                types: vec!["de.ems.grid.*".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    hooks
        .emit(
            &[(Event::ReductionBegan.into(), None)],
            &reduced_home(),
            now,
        )
        .await;
    let pass = {
        let hooks = hooks.clone();
        tokio::spawn(async move { hooks.deliver_due(now).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(hooks.revoke(&created.view.id).await.unwrap());
    pass.await.unwrap();
    assert!(
        hooks.list().await.unwrap().is_empty(),
        "the pass wrote back a subscription the household had revoked"
    );
}

#[tokio::test]
async fn export_limit_lost_device_and_failsafe_exit_each_arrive_once() {
    let (url, received) = receiver().await;
    let hooks = Webhooks::new("home", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    hooks
        .subscribe(
            Asked {
                name: "Recorder".into(),
                url,
                types: vec!["de.ems.*".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    let last = "2026-10-25T01:15:00Z";
    let baseline = serde_json::json!({"reduction":{"failsafe":true}, "devices":[]});
    let observed = serde_json::json!({"devices":[{"id":"ev","notice":"silent","last_seen":last}], "feed_in_limit_w":0.0});
    for snapshot in [&baseline, &observed, &observed] {
        hooks.emit(&[], snapshot, now).await;
    }
    hooks.deliver_due(now).await;
    let events: Vec<serde_json::Value> = received
        .lock()
        .await
        .bodies
        .iter()
        .map(|(_, body)| serde_json::from_slice(body).unwrap())
        .collect();
    assert_eq!(events.len(), 3);
    let event = |kind: &str| events.iter().find(|e| e["type"] == kind).unwrap();
    assert_eq!(event(ems_events::DEVICE_LOST)["subject"], "ev");
    assert_eq!(event(ems_events::DEVICE_LOST)["data"]["last_seen"], last);
    assert_eq!(
        event(ems_events::GRID_LPP_LIMIT_RECEIVED)["data"]["limit_w"],
        0.0
    );
    assert_eq!(
        event(ems_events::GRID_LPC_FAILSAFE_LEFT)["data"]["reduced"],
        false
    );
}

#[tokio::test]
async fn a_restart_drops_expired_deliveries_and_a_reset_erases_the_subscription() {
    let (url, seen) = receiver().await;
    let store = Arc::new(Mutex::new(hemsd::store::Store::in_memory().unwrap()));
    let restart = || Webhooks::new("home", Some(store.clone())).allowing_loopback_for_testing();
    let hooks = restart();
    let now = time::OffsetDateTime::now_utc();
    let created = hooks
        .subscribe(
            Asked {
                name: "Recorder".into(),
                url,
                types: vec!["de.ems.*".into()],
                subjects: vec![],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    let mut day = hemsd::month::ClosedDay {
        day: "2026-11-20".into(),
        money: None,

        forecast: None,
        metered_bill: None,
        comparison: None,
        arbitrage: None,
        imported_wh: 19,
        produced_wh: 0,
        reduced_minutes: None,
        reductions: 0,
        reduction_respected: true,
        feed_in_respected: true,
    };
    hooks.day_closed(&day, now).await;
    day.day = "2026-11-21".into();
    hooks.day_closed(&day, now).await;
    hooks.day_closed(&day, now).await;
    hooks.deliver_due(now).await;
    let restarted = restart();
    restarted.day_closed(&day, now).await;
    restarted
        .emit(
            &[(Event::ReductionBegan.into(), None)],
            &reduced_home(),
            now,
        )
        .await;
    let later = now + time::Duration::hours(25);
    restarted.deliver_due(later).await;
    let received = seen.lock().await;
    assert_eq!(received.bodies.len(), 1);
    let closed: serde_json::Value = serde_json::from_slice(&received.bodies[0].1).unwrap();
    assert_eq!(closed["type"], ems_events::SITE_DAY_CLOSED);
    assert_eq!(closed["data"]["day"], day.day);
    drop(received);
    let log = restarted.deliveries(later).await.unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].outcome, "dropped");
    assert_eq!(log[0].attempt, 0);
    let views = restarted.list().await.unwrap();
    assert_eq!(views[0].owed, 0);
    for value in [
        serde_json::to_string(&views).unwrap(),
        serde_json::to_string(&log).unwrap(),
    ] {
        assert!(!value.contains(created.secret.trim_start_matches("whsec_")));
    }
    assert!(
        restarted
            .deliveries(later + time::Duration::days(8))
            .await
            .unwrap()
            .is_empty()
    );
    restarted
        .emit(
            &[(Event::ReductionEnded.into(), None)],
            &serde_json::json!({}),
            later,
        )
        .await;
    assert_eq!(restarted.list().await.unwrap()[0].owed, 1);
    store.lock().await.factory_reset().unwrap();
    assert!(restart().list().await.unwrap().is_empty());
    assert!(restart().deliveries(later).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_redirect_and_a_stalled_receiver_have_bounded_outcomes() {
    let (target, seen) = receiver().await;
    let app = axum::Router::new()
        .route(
            "/redirect",
            axum::routing::post(
                move || async move { axum::response::Redirect::temporary(&target) },
            ),
        )
        .route(
            "/stalled",
            axum::routing::post(|| async {
                std::future::pending::<axum::http::StatusCode>().await
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let hooks = Webhooks::new("house", None).allowing_loopback_for_testing();
    let now = time::OffsetDateTime::now_utc();
    for path in ["redirect", "stalled"] {
        let added = hooks
            .subscribe(
                Asked {
                    name: path.into(),
                    url: format!("http://{addr}/{path}"),
                    types: vec![ems_events::TEST_PING.into()],
                    subjects: vec![],
                    public: false,
                },
                now,
            )
            .await
            .unwrap();
        let ping = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            hooks.test(&added.view.id, now),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(!ping.delivered);
        if path == "redirect" {
            assert_eq!(ping.status, Some(307));
        } else {
            assert_eq!(ping.status, None);
        }
    }
    assert!(seen.lock().await.bodies.is_empty());
}

#[tokio::test]
async fn only_the_affected_asset_gets_its_goal_or_deviation() {
    use hemsd::runtime::push::{Detector, Signals};
    let (url, seen) = receiver().await;
    let hooks = Webhooks::new("home", None).allowing_loopback_for_testing();
    let mut detector = Detector::default();
    let now = time::OffsetDateTime::now_utc();
    hooks
        .subscribe(
            Asked {
                name: "EV monitor".into(),
                url,
                types: vec!["de.ems.*".into()],
                subjects: vec!["wallbox".into()],
                public: false,
            },
            now,
        )
        .await
        .unwrap();
    let mut home = serde_json::json!({
        "devices":[{"id":"wallbox","kind":"car","commanded_w":3400,"measured_w":0}],
        "goal":[{"target_soc":0.8,"by_minute":420},{"verdict":"feasible"}]
    });
    let _ = detector.changes(&Signals::of(&home), now);
    home["goal"][1] = serde_json::json!({"verdict":"impossible","reachable_soc":0.61});
    home["devices"][0]["notice"] = serde_json::json!("disobedient");
    for minutes in [1, 12, 13] {
        let time = now + time::Duration::minutes(minutes);
        hooks
            .emit(&detector.changes(&Signals::of(&home), time), &home, time)
            .await;
    }
    hooks.deliver_due(now + time::Duration::minutes(13)).await;
    let seen = seen.lock().await;
    assert_eq!(seen.bodies.len(), 2);
    let decode = |i: usize| serde_json::from_slice::<serde_json::Value>(&seen.bodies[i].1).unwrap();
    let goal = decode(0);
    assert_eq!(goal["subject"], "wallbox");
    assert_eq!(goal["data"]["goal"], home["goal"][0]);
    assert_eq!(goal["data"]["reachable"], 0.61);
    let deviation = decode(1);
    assert_eq!(deviation["subject"], "wallbox");
    assert_eq!(deviation["data"]["commanded_w"], 3400.0);
    assert_eq!(deviation["data"]["measured_w"], 0.0);
}
