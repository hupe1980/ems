//! `/v1/push` through the box's real router: who may subscribe a browser, and
//! what the box will and will not take as one.
//!
//! The assembly is `hemsd::runtime::surfaces`, the one `main` serves, because
//! the property under test — a manager's credential does not reach these routes
//! — lives in the layer that assembly adds, not in the handlers.

use std::sync::Arc;

use aws_lc_rs::agreement::{ECDH_P256, PrivateKey};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use tokio::sync::Mutex;

const TOKEN: &str = "the-households-own-credential";

/// A box on loopback, its address and a connected manager's credential.
async fn a_box(with_store: bool) -> (String, String) {
    let household = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household is a valid site");
    let site = household.site.id.to_string();
    let store = Arc::new(Mutex::new(
        hemsd::store::Store::in_memory().expect("a store"),
    ));
    let access = hemsd::runtime::access::LocalAccess::for_testing(&site, TOKEN)
        .remembering(Some(store.clone()));
    let manager = access
        .connect_manager("aggregator-nord")
        .await
        .expect("a manager can be connected");
    let push = if with_store {
        Some(
            hemsd::runtime::push::Push::open(store)
                .await
                .expect("a sender"),
        )
    } else {
        None
    };
    let local = hemsd::runtime::api::Local::new(
        Arc::new(Mutex::new(hemsd::runtime::control::Status::default())),
        site,
        None,
        hemsd::runtime::overrides::Overrides::new(),
        None,
        access.clone(),
        None,
    )
    .with_push(push);
    let router = hemsd::runtime::surfaces(local, None, access);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("its address").to_string();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (address, manager)
}

/// What a browser's `PushSubscription.toJSON()` looks like, for `endpoint`.
fn a_browser(endpoint: &str) -> serde_json::Value {
    let key = PrivateKey::generate(&ECDH_P256).expect("a key");
    let public = key.compute_public_key().expect("a point");
    serde_json::json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": {
            "p256dh": URL_SAFE_NO_PAD.encode(public.as_ref()),
            "auth": URL_SAFE_NO_PAD.encode([7_u8; 16]),
        },
        "lang": "en",
        "events": ["reduction", "goal"],
        "quiet": {"from_minute": 1320, "to_minute": 420},
    })
}

#[tokio::test]
async fn a_household_subscribes_changes_and_unsubscribes_a_browser() {
    let (address, _) = a_box(true).await;
    let url = format!("http://{address}/v1/push");
    let client = reqwest::Client::new();

    let overview: serde_json::Value = client
        .get(&url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = URL_SAFE_NO_PAD
        .decode(overview["key"].as_str().unwrap())
        .expect("base64url, the spelling `applicationServerKey` takes");
    assert_eq!((key.len(), key[0]), (65, 4), "an uncompressed P-256 point");
    assert_eq!(overview["subscriptions"], serde_json::json!([]));
    assert_eq!(overview["at_most"], 8);

    let endpoint = "https://web.push.apple.com/QGuZ-example";
    let browser = a_browser(endpoint);
    let taken = client
        .post(&url)
        .bearer_auth(TOKEN)
        .header("Origin", "https://hems.local")
        .json(&browser)
        .send()
        .await
        .unwrap();
    assert_eq!(taken.status(), reqwest::StatusCode::OK);
    let view: serde_json::Value = taken.json().await.unwrap();
    assert_eq!(view["endpoint"], endpoint);
    assert_eq!(view["lang"], "en");
    assert_eq!(view["events"], serde_json::json!(["reduction", "goal"]));
    assert_eq!(
        view["quiet"],
        serde_json::json!({"from_minute": 1320, "to_minute": 420})
    );

    // Listed, and never with the keys that would let a reader of this answer
    // decrypt what the box sends that browser.
    let listed = client
        .get(&url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(listed.contains(endpoint));
    assert!(!listed.contains("p256dh") && !listed.contains("auth\""));
    assert!(!listed.contains(browser["keys"]["p256dh"].as_str().unwrap()));

    // The same endpoint again is a change, not a second subscription.
    let mut changed = browser.clone();
    changed["events"] = serde_json::json!(["failsafe"]);
    changed["quiet"] = serde_json::Value::Null;
    client
        .post(&url)
        .bearer_auth(TOKEN)
        .json(&changed)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let overview: serde_json::Value = client
        .get(&url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(overview["subscriptions"].as_array().unwrap().len(), 1);
    assert_eq!(
        overview["subscriptions"][0]["events"],
        serde_json::json!(["failsafe"])
    );
    assert_eq!(
        overview["subscriptions"][0]["quiet"],
        serde_json::Value::Null
    );

    let which = serde_json::json!({ "endpoint": endpoint });
    let forget = || client.delete(&url).bearer_auth(TOKEN).json(&which).send();
    assert_eq!(
        forget().await.unwrap().status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        forget().await.unwrap().status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(format!("{url}/test"))
            .bearer_auth(TOKEN)
            .json(&which)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND,
        "a test goes only to a browser that is subscribed"
    );
}

#[tokio::test]
async fn the_box_will_not_be_pointed_at_the_household_s_own_network() {
    let (address, _) = a_box(true).await;
    let url = format!("http://{address}/v1/push");
    let client = reqwest::Client::new();
    for endpoint in [
        "https://192.168.178.1/upnp/control",
        "http://web.push.apple.com/plain",
        "https://localhost:8443/v1/pairing",
        "https://fcm.googleapis.com.example.net/x",
    ] {
        let refused = client
            .post(&url)
            .bearer_auth(TOKEN)
            .json(&a_browser(endpoint))
            .send()
            .await
            .unwrap();
        assert_eq!(
            refused.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "{endpoint}"
        );
        assert!(
            refused.text().await.unwrap().contains("push service"),
            "the refusal says why"
        );
    }
    let mut bad_keys = a_browser("https://web.push.apple.com/x");
    bad_keys["keys"]["p256dh"] = "bm90IGEgcG9pbnQ".into();
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(TOKEN)
            .json(&bad_keys)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let overview: serde_json::Value = client
        .get(&url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(overview["subscriptions"], serde_json::json!([]));
}

#[tokio::test]
async fn an_energy_manager_s_credential_does_not_reach_notifications() {
    let (address, manager) = a_box(true).await;
    let url = format!("http://{address}/v1/push");
    let client = reqwest::Client::new();
    let body = a_browser("https://web.push.apple.com/the-managers-own-phone");
    let which = serde_json::json!({ "endpoint": "https://web.push.apple.com/x" });

    let attempts = [
        client.get(&url),
        client.post(&url).json(&body),
        client.delete(&url).json(&which),
        client.post(format!("{url}/test")).json(&which),
    ];
    for attempt in attempts {
        let attempt = attempt.build().unwrap();
        let what = format!("{} {}", attempt.method(), attempt.url().path());
        let as_manager = client
            .execute({
                let mut r = attempt.try_clone().unwrap();
                r.headers_mut().insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {manager}").parse().unwrap(),
                );
                r
            })
            .await
            .unwrap();
        assert_eq!(
            as_manager.status(),
            reqwest::StatusCode::FORBIDDEN,
            "{what}: a manager that could subscribe would be told when this \
             household's car will not be ready"
        );
        let as_nobody = client.execute(attempt).await.unwrap();
        assert_eq!(
            as_nobody.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_box_with_no_store_takes_no_subscription() {
    let (address, _) = a_box(false).await;
    let url = format!("http://{address}/v1/push");
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(TOKEN)
            .json(&a_browser("https://web.push.apple.com/x"))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_service_worker_is_served_at_the_root_without_a_credential() {
    let (address, _) = a_box(true).await;
    let served = reqwest::get(format!("http://{address}/sw.js"))
        .await
        .unwrap();
    assert_eq!(served.status(), reqwest::StatusCode::OK);
    assert!(
        served.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"),
        "a browser registers a worker only from a JavaScript type"
    );
    assert!(served.headers().contains_key("content-security-policy"));
    assert!(served.text().await.unwrap().contains("showNotification"));
}
