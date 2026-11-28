//! The collector, over the wire, with the report a box actually sends.
//!
//! `hemsd` produces a `DayKpis`; this service consumes one. The type is shared,
//! so the two cannot drift — but a shared type still leaves a wire, and this is
//! the test that says the wire works.
//!
//! The wire is a **signed CloudEvent**, so every request here is signed
//! the way a box signs one, and three of the tests are about what happens when
//! it is not.

use ems_core::prelude::CostBreakdown;
use ems_core::report::DayKpis;
use obsd::api::{Observed, router};
use time::macros::date;

/// A distinct deterministic key for each fixture household.
fn key_for(site: &str) -> ems_events::webhook::Webhook {
    ems_events::webhook::Webhook::from_bytes(format!("test-signing-key-for-{site}").into_bytes())
        .unwrap()
}
/// The credential an operator reads the fleet view with.
const OPERATOR: &str = "tok-operator";

/// A day a box would send after a reduction it respected.
///
/// `day` is **how many days ago**, not a calendar date, and that is not a
/// stylistic choice: the fleet view is a *window* — `keep_days` back from today
/// — because the retention sweep deletes outside the same window (D157). A
/// fixture on a fixed date would drift out of it and the test would start
/// failing on a Tuesday for a reason nobody changed.
fn good_day(site: &str, days_ago: u8) -> DayKpis {
    DayKpis {
        site: site.into(),
        date: time::OffsetDateTime::now_utc().date() - time::Duration::days(i64::from(days_ago)),
        imported_kwh: 55.7,
        exported_kwh: 0.3,
        produced_kwh: 8.4,
        self_sufficiency: 0.13,
        economics: Some(ems_core::report::Economics {
            cost: CostBreakdown {
                energy_eur: 21.08,
                wear_eur: 0.62,
                discomfort_eur: 0.19,
                stored_eur: 0.14,
                ..CostBreakdown::default()
            },
            baseline: CostBreakdown {
                energy_eur: 24.12,
                ..CostBreakdown::default()
            },
        }),
        respected_the_grid: true,
        control_events: 1,
        forecast: Some(ems_core::report::ForecastScores {
            pv_coverage: 0.81,

            pv_crps: 192.0,

            load_coverage: 0.85,

            load_crps: 18.0,
        }),
        ..DayKpis::default()
    }
}

/// Start the service on an ephemeral port; returns the address and the stopper.
async fn start() -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    start_with_a_site_credential(None).await
}

/// The same service, additionally accepting one household's own credential.
///
/// The default harness configures an operator and nothing else, which is why
/// nothing here noticed what a *site* token could reach.
async fn start_with_a_site_credential(
    site: Option<(&str, &str)>,
) -> (std::net::SocketAddr, ems_service::shutdown::ShutdownTrigger) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bound = listener.local_addr().unwrap();
    drop(listener);
    let settings = ems_service::Settings {
        listen: bound,
        shutdown_grace_s: 2,
        ..ems_service::Settings::default()
    };
    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let store = obsd::store::Store::new(fixture.db.clone());
    // Leaked on purpose: the pool outlives the server task, and a test process
    // ends when the test does.
    std::mem::forget(fixture);
    let (signal, trigger) = ems_service::Shutdown::channel();
    let server = ems_service::Server::new(
        ems_service::identity!(),
        settings,
        ems_service::Health::new(),
        router(
            Observed::new(
                store,
                60,
                time::Duration::days(2),
                // Every site the tests report for, each with a key of its own — which
                // is the point: two sites sharing one key would make "who signed this"
                // unanswerable, and the daemon refuses that configuration.
                (0..10)
                    .map(|i| format!("site-{i}"))
                    .chain(["haus-1".to_owned(), "haus-2".to_owned()])
                    .map(|site| {
                        let key = key_for(&site);
                        (site, key)
                    })
                    .collect(),
                ems_service::Credentials::resolve(
                    &site
                        .map(|(name, token)| (name.to_owned(), ems_service::Secret::literal(token)))
                        .into_iter()
                        .collect(),
                    &std::collections::BTreeMap::new(),
                    &[ems_service::OperatorCredential {
                        token: ems_service::Secret::literal(OPERATOR),
                        tenant: "*".into(),
                    }],
                )
                .unwrap(),
            )
            .with_customers(
                ems_service::Credentials::default()
                    .with_site("haus-1", "customer-1")
                    .with_site("haus-2", "customer-2"),
            ),
        ),
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
    body: Option<&str>,
    headers: &[(&str, String)],
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let extra: String = headers
        .iter()
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect();
    let head = match body {
        Some(b) => format!(
            "{method} {path} HTTP/1.1\r\nHost: h\r\nContent-Type: application/cloudevents+json\r\n\
             {extra}Content-Length: {}\r\nConnection: close\r\n\r\n{b}",
            b.len()
        ),
        None => {
            format!("{method} {path} HTTP/1.1\r\nHost: h\r\n{extra}Connection: close\r\n\r\n")
        }
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

/// The body and headers a box sends for one day.
fn signed_report(
    day: &DayKpis,
    secret: &ems_events::webhook::Webhook,
    at: time::OffsetDateTime,
) -> (String, Vec<(&'static str, String)>) {
    let event = ems_events::Event::new(
        ems_events::SITE_DAY_REPORTED,
        format!("ems://sites/{}", day.site),
        format!("{}:{}", day.site, day.date),
        at,
        day.clone(),
    )
    .about(day.date.to_string());
    let body = String::from_utf8(event.to_bytes().unwrap()).unwrap();
    let signature = ems_events::webhook::sign(secret, &event.id, at, body.as_bytes()).unwrap();
    (body, signature.headers().to_vec())
}

/// The same, at this instant and under the fleet's own secret.
fn report_now(day: &DayKpis) -> (String, Vec<(&'static str, String)>) {
    signed_report(day, &key_for(&day.site), time::OffsetDateTime::now_utc())
}

/// The header an operator reads with.
fn operator() -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {OPERATOR}"))]
}

/// `POST /v1/days` with a report a box would have signed.
async fn post_day(address: std::net::SocketAddr, day: &DayKpis) -> u16 {
    let (body, headers) = report_now(day);
    request(address, "POST", "/v1/days", Some(&body), &headers)
        .await
        .0
}

#[tokio::test]
async fn a_day_reported_over_the_wire_reaches_the_summary() {
    let (address, trigger) = start().await;
    assert_eq!(post_day(address, &good_day("site-1", 15)).await, 202);

    let (status, summary) = request(address, "GET", "/v1/fleet", None, &operator()).await;
    assert_eq!(status, 200);
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["sites"], 1);
    assert_eq!(summary["measured_days"], 1);
    // €24,12 − €22,03 = €2,09, which is the reference winter day's own saving.
    let saving = summary["saving_eur"].as_f64().unwrap();
    assert!((saving - 2.09).abs() < 0.005, "{saving}");
    trigger.trigger();
}

#[tokio::test]
async fn a_breach_arrives_as_a_named_finding_and_not_as_a_percentage() {
    let (address, trigger) = start().await;
    for i in 0..5 {
        post_day(address, &good_day(&format!("site-{i}"), 15)).await;
    }
    let mut bad = good_day("site-9", 15);
    bad.respected_the_grid = false;
    bad.worst_overshoot_w = 850.0;
    post_day(address, &bad).await;

    let (_, summary) = request(address, "GET", "/v1/fleet", None, &operator()).await;
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    let breached = summary["breached"].as_array().unwrap();
    assert_eq!(breached.len(), 1);
    assert_eq!(breached[0]["site"], "site-9");
    let fifteen_days_ago = (time::OffsetDateTime::now_utc().date() - time::Duration::days(15))
        .format(&time::format_description::well_known::Iso8601::DATE)
        .expect("an ISO date");
    assert_eq!(breached[0]["date"], fifteen_days_ago);
    assert!(breached[0]["detail"].as_str().unwrap().contains("850"));
    trigger.trigger();
}

#[tokio::test]
async fn a_site_can_be_asked_about_on_its_own() {
    let (address, trigger) = start().await;
    for day in 15..18 {
        post_day(address, &good_day("site-1", day)).await;
    }
    let (status, days) = request(address, "GET", "/v1/sites/site-1", None, &operator()).await;
    assert_eq!(status, 200);
    let days: Vec<DayKpis> = serde_json::from_str(&days).unwrap();
    assert_eq!(days.len(), 3);
    // Oldest first, which is what a chart wants and what the store's own
    // ordering already is.
    let today = time::OffsetDateTime::now_utc().date();
    assert_eq!(days[0].date, today - time::Duration::days(17));

    let (status, _) = request(address, "GET", "/v1/sites/nobody", None, &operator()).await;
    assert_eq!(status, 404);
    trigger.trigger();
}

#[tokio::test]
async fn a_report_with_a_field_this_build_does_not_know_is_refused() {
    // `deny_unknown_fields`, and it is the whole point of sharing the type: a
    // box running a newer build that renamed a field must fail loudly here
    // rather than have the fleet silently default it and average a zero.
    //
    // It is signed correctly, so this is a `400` and not a `401`: the box is who
    // it says it is and is sending something this build cannot read, which is a
    // different problem from an intruder and deserves a different answer.
    let (address, trigger) = start().await;
    let (body, headers) = report_now(&good_day("site-1", 15));
    let mut event: serde_json::Value = serde_json::from_str(&body).unwrap();
    event["data"]["invented_field"] = serde_json::json!(1);
    let tampered = serde_json::to_string(&event).unwrap();
    // Re-sign it: the point of this test is the schema, not the signature.
    let at = time::OffsetDateTime::now_utc();
    let id = headers[0].1.clone();
    let key = key_for("site-1");
    let signature = ems_events::webhook::sign(&key, &id, at, tampered.as_bytes()).unwrap();
    let (status, _) = request(
        address,
        "POST",
        "/v1/days",
        Some(&tampered),
        &signature.headers(),
    )
    .await;
    assert_eq!(
        status, 400,
        "a schema the fleet does not understand is refused"
    );
    trigger.trigger();
}

#[tokio::test]
async fn an_unsigned_report_is_refused() {
    // What the signature is for: without it, anybody who can reach this
    // endpoint can write a household into — or out of — the list of sites that
    // did not respect a network operator's reduction.
    let (address, trigger) = start().await;
    let (body, _) = report_now(&good_day("site-1", 15));
    let (status, _) = request(address, "POST", "/v1/days", Some(&body), &[]).await;
    assert_eq!(status, 401);

    let (_, summary) = request(address, "GET", "/v1/fleet", None, &operator()).await;
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["sites"], 0, "a refused report must not be recorded");
    trigger.trigger();
}

#[tokio::test]
async fn a_report_edited_after_signing_is_refused() {
    // Not the box's build being newer — the body being changed on the way. The
    // signature is over the bytes, so a compliant day rewritten into a breach
    // does not verify.
    let (address, trigger) = start().await;
    let (body, headers) = report_now(&good_day("site-1", 15));
    let edited = body.replace(
        "\"respected_the_grid\":true",
        "\"respected_the_grid\":false",
    );
    assert_ne!(edited, body, "the field has to be in the body to be edited");
    let (status, _) = request(address, "POST", "/v1/days", Some(&edited), &headers).await;
    assert_eq!(status, 401);
    trigger.trigger();
}

#[tokio::test]
async fn a_captured_report_stops_working() {
    // Replay: the exact bytes and the exact signature a box sent, six minutes
    // later. Without the timestamp inside the signed content, re-sending
    // yesterday's breach every hour would be a supported operation.
    let (address, trigger) = start().await;
    let day = good_day("site-1", 15);
    let stale = time::OffsetDateTime::now_utc() - time::Duration::minutes(6);
    let (body, headers) = signed_report(&day, &key_for("site-1"), stale);
    let (status, _) = request(address, "POST", "/v1/days", Some(&body), &headers).await;
    assert_eq!(status, 401);
    trigger.trigger();
}

#[tokio::test]
async fn an_event_of_another_type_does_not_become_a_day() {
    // A correctly signed message from a box, of a type this endpoint does not
    // read. Signed by us and still not a day report — the two checks are
    // independent and both have to hold.
    let (address, trigger) = start().await;
    let at = time::OffsetDateTime::now_utc();
    let event = ems_events::Event::new(
        ems_events::SITE_PLAN_PUBLISHED,
        "ems://sites/site-1",
        "site-1:plan",
        at,
        good_day("site-1", 15),
    );
    let body = String::from_utf8(event.to_bytes().unwrap()).unwrap();
    let key = key_for("site-1");
    let signature = ems_events::webhook::sign(&key, &event.id, at, body.as_bytes()).unwrap();
    let (status, _) = request(
        address,
        "POST",
        "/v1/days",
        Some(&body),
        &signature.headers(),
    )
    .await;
    assert_eq!(status, 400);
    trigger.trigger();
}

#[tokio::test]
async fn the_fleet_view_is_not_served_without_a_credential() {
    // `/v1/fleet` carries what every household spent and drew, and the named
    // list of those that did not respect a network operator's reduction. Writing
    // is authenticated by a signature; reading is a different caller and needs
    // its own credential.
    let (address, trigger) = start().await;
    post_day(address, &good_day("site-1", 15)).await;
    for path in ["/v1/fleet", "/v1/sites/site-1"] {
        assert_eq!(
            request(address, "GET", path, None, &[]).await.0,
            401,
            "{path}"
        );
        assert_eq!(
            request(
                address,
                "GET",
                path,
                None,
                &[("authorization", "Bearer tok-invented".to_owned())]
            )
            .await
            .0,
            401,
            "{path}"
        );
    }
    trigger.trigger();
}

#[tokio::test]
async fn one_household_cannot_read_the_whole_fleet() {
    // `/v1/fleet` carries `breached` — the named list of households that did not
    // respect a network operator's reduction — and `below_minimum`. A site's own
    // box credential says "I am this household"; it says nothing about any
    // other, and a summary over all of them is not this household's data.
    let (address, trigger) = start_with_a_site_credential(Some(("haus-1", "tok-haus-1"))).await;

    let own = [("authorization", "Bearer tok-haus-1".to_owned())];
    let (status, _) = request(address, "GET", "/v1/fleet", None, &own).await;
    assert_eq!(
        status, 403,
        "a household's own token must not read the fleet summary"
    );

    // Its own days, on the other hand, are exactly what it may read.
    let (status, _) = request(address, "GET", "/v1/sites/haus-1", None, &own).await;
    assert_ne!(status, 403, "and it is not locked out of its own record");

    // …and another household's are not.
    let (status, _) = request(address, "GET", "/v1/sites/haus-2", None, &own).await;
    assert_eq!(status, 403);

    let (status, _) = request(address, "GET", "/v1/fleet", None, &operator()).await;
    assert_eq!(status, 200, "the operator still reads it");

    trigger.trigger();
}

#[tokio::test]
async fn a_box_cannot_report_a_day_as_another_household() {
    // The signature says the bytes were not edited. It does **not** say who sent
    // them, because the secret is the fleet's rather than the household's — so
    // any box that can sign can attribute a day to any site. What that buys an
    // attacker is the one thing `obsd` exists to hold: `breached` is the named
    // list of households that did not respect a network operator's reduction,
    // and a forged day writes to it.
    let (address, trigger) = start().await;

    // `haus-1`'s box, holding the fleet secret, reports a breach for `haus-2`.
    let mut forged = good_day("haus-2", 15);
    forged.respected_the_grid = false;
    let (body, headers) =
        signed_report(&forged, &key_for("haus-1"), time::OffsetDateTime::now_utc());
    let (status, _) = request(address, "POST", "/v1/days", Some(&body), &headers).await;
    assert_eq!(
        status, 401,
        "a box may report its own day and nobody else's — a signature over a \
         shared secret authenticates the bytes, never the sender"
    );

    // And the fleet view did not learn about a breach nobody committed.
    let (status, summary) = request(address, "GET", "/v1/fleet", None, &operator()).await;
    assert_eq!(status, 200);
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(
        summary["breached"].as_array().map(Vec::len),
        Some(0),
        "nothing was recorded: {summary}"
    );

    // Its own day, under its own secret, is taken.
    let (body, headers) = signed_report(
        &good_day("haus-1", 15),
        &key_for("haus-1"),
        time::OffsetDateTime::now_utc(),
    );
    let (status, _) = request(address, "POST", "/v1/days", Some(&body), &headers).await;
    assert_eq!(status, 202);

    trigger.trigger();
}

/// The retention window is a `DELETE` an operator can ask questions of, and the
/// record survives the process that wrote it (D157).
#[tokio::test]
async fn the_window_is_bounded_and_the_oldest_days_go_first() {
    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let store = obsd::store::Store::new(fixture.db.clone());
    let at = time::OffsetDateTime::now_utc();
    for d in 0..6_i64 {
        let day = ems_core::report::DayKpis {
            site: "a".to_owned(),
            date: date!(2026 - 02 - 20) + time::Duration::days(d),
            ..ems_core::report::DayKpis::default()
        };
        store.record(&day, at).await.expect("a day");
    }

    assert_eq!(store.prune(date!(2026 - 02 - 23)).await.unwrap(), 3);
    let left = store
        .history(date!(2026 - 01 - 01), &ems_service::SiteScope::Every)
        .await
        .unwrap();
    let days = &left["a"].days;
    assert_eq!(days.len(), 3, "the three newest");
    assert_eq!(*days.keys().next().unwrap(), date!(2026 - 02 - 23));

    // …and a re-report of a day already on record is a correction, not a second
    // day. The primary key is the rule, so a fleet cannot double one
    // household's saving inside an average.
    let again = ems_core::report::DayKpis {
        site: "a".to_owned(),
        date: date!(2026 - 02 - 25),
        ..ems_core::report::DayKpis::default()
    };
    store.record(&again, at).await.expect("a correction");
    let left = store
        .history(date!(2026 - 01 - 01), &ems_service::SiteScope::Every)
        .await
        .unwrap();
    assert_eq!(left["a"].days.len(), 3, "still three");
}

/// A restart does not lose the fleet.
///
/// The defect this whole change exists for: every day report `obsd` had ever
/// accepted lived in one process's memory, so a restart — or a rescheduled pod —
/// discarded the named list of households that did not respect a network
/// operator's reduction, with no error anywhere. A box reports a day once, so
/// what was lost was lost.
#[tokio::test]
async fn the_fleets_record_outlives_the_process_that_collected_it() {
    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let at = time::OffsetDateTime::now_utc();
    {
        let store = obsd::store::Store::new(fixture.db.clone());
        let day = ems_core::report::DayKpis {
            site: "haus-1".to_owned(),
            date: date!(2026 - 02 - 25),
            respected_the_grid: false,
            worst_overshoot_w: 1_200.0,
            ..ems_core::report::DayKpis::default()
        };
        store.record(&day, at).await.expect("a breach");
    }

    // A different collector, over the same database.
    let store = obsd::store::Store::new(fixture.db.clone());
    let read = store
        .history(date!(2026 - 01 - 01), &ems_service::SiteScope::Every)
        .await
        .unwrap();
    let fleet = obsd::fleet::Fleet::of(
        read.into_iter()
            .map(|(site, days)| (site, days.into()))
            .collect(),
    );
    let summary = fleet.summarise(at, time::Duration::days(2));
    assert_eq!(summary.sites, 1);
    assert_eq!(
        summary.breached.len(),
        1,
        "the finding survived the process that collected it"
    );
    assert_eq!(summary.breached[0].site, "haus-1");
}

/// A scope reaches the **query**, not just the summary.
///
/// D112 says an aggregate is computed *within* the caller's scope rather than
/// filtered afterwards, and for as long as the fleet was a map in memory that
/// distinction cost nothing. It is a database now, and reading every household's
/// day reports through one tenant's request to discard most of them afterwards
/// is exactly what that decision forbids — so the scope is a `WHERE` clause, and
/// this is the test that fails if it stops being one.
#[tokio::test]
async fn one_tenants_read_does_not_pull_anothers_rows() {
    use std::collections::BTreeSet;
    use time::macros::date;

    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let store = obsd::store::Store::new(fixture.db.clone());
    let at = time::OffsetDateTime::now_utc();
    for site in ["nord-1", "sued-1"] {
        let day = ems_core::report::DayKpis {
            site: site.to_owned(),
            date: date!(2026 - 02 - 25),
            ..ems_core::report::DayKpis::default()
        };
        store.record(&day, at).await.expect("a day");
    }

    let nord = ems_service::SiteScope::Tenant {
        name: "nord".into(),
        sites: BTreeSet::from(["nord-1".to_owned()]),
    };
    let read = store
        .history(date!(2026 - 01 - 01), &nord)
        .await
        .expect("the tenant's window");
    assert_eq!(
        read.keys().collect::<Vec<_>>(),
        vec!["nord-1"],
        "the other tenant's rows never left the database"
    );

    // …and one household reads exactly one household.
    let one = ems_service::SiteScope::One("sued-1".to_owned());
    let read = store.history(date!(2026 - 01 - 01), &one).await.unwrap();
    assert_eq!(read.len(), 1);
    assert!(read.contains_key("sued-1"));

    // `Every` is the only scope that reads everything, and it is a variant
    // somebody had to configure.
    let all = store
        .history(date!(2026 - 01 - 01), &ems_service::SiteScope::Every)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

/// The scoped read is an **index** scan, not a scan with a filter on it.
///
/// The companion to `one_tenants_read_does_not_pull_anothers_rows`, which proves
/// the rows do not come back. This proves they are not *read*: a predicate that
/// the planner applies after a sequential scan keeps the answer correct and
/// reads the whole fleet's window to produce it, which on a shared deployment is
/// one tenant's request touching every other tenant's pages.
#[tokio::test]
async fn a_scoped_read_is_an_index_scan() {
    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let client = fixture.db.get().await.expect("a connection");
    // Enough rows that a sequential scan is not the cheapest plan by accident.
    client
        .batch_execute(
            "INSERT INTO site_day
             SELECT 'site-' || (g % 500), '2026-01-01'::date + (g / 500), '{}'::jsonb, now()
             FROM generate_series(1, 30000) g ON CONFLICT DO NOTHING;
             ANALYZE site_day;",
        )
        .await
        .expect("a loaded table");

    let rows = client
        .query(
            "EXPLAIN (COSTS OFF)
             SELECT site, day FROM site_day
             WHERE day >= '2026-01-01'::date
               AND (ARRAY['site-3']::text[] IS NULL OR site = ANY(ARRAY['site-3']::text[]))
             ORDER BY site, day",
            &[],
        )
        .await
        .expect("a plan");
    let plan: String = rows
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("site_day_pkey"),
        "a tenant's window left the primary key:\n{plan}"
    );
    assert!(
        !plan.contains("Seq Scan"),
        "one tenant's request must not read every tenant's pages:\n{plan}"
    );
}

/// The retention sweep does not read the whole window to delete one day of it.
///
/// The primary key leads with `site`, so it cannot answer `day < $1` across
/// every household — and this test exists because the index that can was once
/// deleted along with the comment above it, and every other test still passed.
/// A missing index is not a failure, it is the same answer read a thousand times
/// more expensively.
#[tokio::test]
async fn the_retention_sweep_is_an_index_scan() {
    let fixture = ems_service::testdb::Postgres::start(obsd::store::MIGRATIONS).await;
    let client = fixture.db.get().await.expect("a connection");
    client
        .batch_execute(
            "INSERT INTO site_day
             SELECT 'site-' || (g % 500), '2026-01-01'::date + (g / 500), '{}'::jsonb, now()
             FROM generate_series(1, 30000) g ON CONFLICT DO NOTHING;
             ANALYZE site_day;",
        )
        .await
        .expect("a loaded table");

    // A cutoff that removes a small slice, which is what a daily sweep over a
    // sixty-day window actually does. Asking to delete most of the table
    // correctly plans as a sequential scan.
    let rows = client
        .query(
            "EXPLAIN (COSTS OFF) DELETE FROM site_day WHERE day < '2026-01-03'::date",
            &[],
        )
        .await
        .expect("a plan");
    let plan: String = rows
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("site_day_by_day"),
        "the retention sweep left its index:\n{plan}"
    );
}

#[tokio::test]
async fn customer_receipt_requires_own_evidence_and_preserves_the_close() {
    let (addr, shutdown) = start().await;
    let departure = time::macros::datetime!(2026-01-15 07:00 UTC);
    let submit = |site: &'static str, at, closing_wh, requested_wh| async move {
        let data = ems_events::departure::SimulatedDeparture {
            departure: at,
            requested_wh,
            opening_wh: 1000,
            rejoin_wh: 1600,
            closing_wh,
        };
        let event = ems_events::Event::new(
            ems_events::SITE_DEPARTURE_REHEARSED,
            format!("ems://sites/{site}"),
            "receipt",
            at,
            data,
        );
        let body = event.to_bytes().unwrap();
        let sig = ems_events::webhook::sign(
            &key_for("haus-1"),
            "receipt",
            time::OffsetDateTime::now_utc(),
            &body,
        )
        .unwrap();
        request(
            addr,
            "POST",
            "/v1/departures",
            Some(std::str::from_utf8(&body).unwrap()),
            &sig.headers(),
        )
        .await
        .0
    };
    let read = |token: &'static str, path: &'static str| async move {
        request(
            addr,
            "GET",
            path,
            None,
            &[("authorization", format!("Bearer {token}"))],
        )
        .await
    };
    let path = "/v1/me/departure";
    assert_eq!(read(OPERATOR, path).await.0, 401);
    assert_eq!(read("customer-1", "/v1/fleet").await.0, 401);
    assert_eq!(submit("haus-2", departure, None, 1000).await, 401);
    assert_eq!(submit("haus-1", departure, Some(1200), 1000).await, 422);
    assert_eq!(submit("haus-1", departure, None, 1000).await, 204);
    assert_eq!(
        read("customer-2", "/v1/me/departure?site=haus-1").await.0,
        204
    );
    let value = |s: String| serde_json::from_str::<serde_json::Value>(&s).unwrap();
    assert!(value(read("customer-1", path).await.1)["delivered_wh"].is_null());
    assert_eq!(submit("haus-1", departure, Some(1800), 1000).await, 204);
    assert_eq!(submit("haus-1", departure, None, 1000).await, 204);
    assert_eq!(submit("haus-1", departure, Some(2000), 1000).await, 409);
    assert_eq!(submit("haus-1", departure, Some(1800), 700).await, 409);
    assert_eq!(value(read("customer-1", path).await.1)["shortfall_wh"], 200);
    let later = departure + time::Duration::days(1);
    assert_eq!(submit("haus-1", later, Some(2000), 1000).await, 204);
    assert_eq!(submit("haus-1", departure, None, 1000).await, 409);
    assert_eq!(value(read("customer-1", path).await.1)["shortfall_wh"], 0);
    shutdown.trigger();
}
