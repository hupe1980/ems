//! Commissioned sites use MeterStore for receipts, replay, settlement reads and retention.
mod common;
use ems_core::prelude::Slot;
use ems_grid::mispel::QuarterHour;
use ems_service::testdb::Postgres;
use histd::{Store, StoreError, config::RegisterMeter};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use time::{Duration, macros::datetime};

#[tokio::test]
async fn commissioned_registers_keep_exact_revisions_at_the_receipt_boundary() {
    let pg = Postgres::start(histd::store::MIGRATIONS).await;
    let database = common::database(&pg);
    let meter = RegisterMeter {
        malo: "12345678905".parse().unwrap(),
        operator: "9900000000003".parse().unwrap(),
    };
    let sites = BTreeMap::from([("one".to_owned(), meter.clone()), ("two".to_owned(), meter)]);
    let store = Store::new(pg.db.clone())
        .with_register_meters(&database, sites.clone())
        .await
        .unwrap();
    let at = datetime!(2026-11-01 00:00 UTC);
    let receipt = at + Duration::DAY;
    let mut q = QuarterHour::empty(Slot::containing(at));
    q.grid_draw = Decimal::new(1, 7);
    q.storage_consumption = Some(Decimal::ZERO);
    store.put_quarter_hour("one", &q, receipt).await.unwrap();
    let mut changed = q;
    changed.spot_price = Some(Decimal::new(-1, 8));
    changed.storage_consumption = None;
    let next = receipt + Duration::HOUR;
    store.put_quarter_hour("one", &changed, next).await.unwrap();
    store.put_quarter_hour("one", &changed, next).await.unwrap();
    store.put_quarter_hour("two", &q, next).await.unwrap();
    assert!(
        store
            .quarter_hours_as_of("one", None, None, Some(receipt - Duration::MICROSECOND))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .quarter_hours_as_of("one", None, None, Some(next - Duration::MICROSECOND))
            .await
            .unwrap(),
        [q]
    );
    assert_eq!(
        store
            .quarter_hours_as_of("one", None, None, Some(next))
            .await
            .unwrap(),
        [changed]
    );
    let other = QuarterHour::empty(Slot::containing(at - Duration::minutes(15)));
    assert!(matches!(
        store.put_quarter_hours("one", &[other, q], next).await,
        Err(StoreError::ConflictingAssertion(_))
    ));
    let mut excessive = changed;
    excessive.grid_draw = Decimal::new(1, 8);
    assert!(matches!(
        store
            .put_quarter_hour("one", &excessive, next + Duration::HOUR)
            .await,
        Err(StoreError::InvalidAssertion(_))
    ));
    assert!(
        Store::new(pg.db.clone())
            .with_register_meters(&database, BTreeMap::new())
            .await
            .is_err()
    );
    let mut changed_identity = sites.clone();
    changed_identity.get_mut("one").unwrap().malo = "41373559241".parse().unwrap();
    assert!(
        Store::new(pg.db.clone())
            .with_register_meters(&database, changed_identity)
            .await
            .is_err()
    );
    drop(store);
    let reopened = Store::new(pg.db.clone())
        .with_register_meters(&database, sites)
        .await
        .unwrap();
    assert_eq!(
        reopened.quarter_hours("one", None, None).await.unwrap(),
        [changed]
    );
    assert_eq!(
        reopened.quarter_hours("two", None, None).await.unwrap(),
        [q]
    );
    let sql_count: i64 = pg
        .db
        .get()
        .await
        .unwrap()
        .query_one("SELECT count(*) FROM quarter_hour", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sql_count, 0);
    reopened
        .prune(at + histd::store::EVIDENCE_RETENTION)
        .await
        .unwrap();
    assert!(
        reopened
            .quarter_hours("one", None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn settlement_reads_the_commissioned_month_at_each_cutoff() {
    use ems_grid::mispel::Basisfall;
    use histd::config::MispelSettings;
    let pg = Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(pg.db.clone())
        .with_register_meters(
            &common::database(&pg),
            BTreeMap::from([(
                "meter".into(),
                RegisterMeter {
                    malo: "12345678905".parse().unwrap(),
                    operator: "9900000000003".parse().unwrap(),
                },
            )]),
        )
        .await
        .unwrap();
    let start = datetime!(2026-10-31 23:00 UTC);
    let receipt = datetime!(2026-12-01 06:00 UTC);
    let rows: Vec<_> = (0..2880)
        .map(|i| QuarterHour {
            slot: Slot::containing(start + Duration::minutes(i * 15)),
            grid_draw: Decimal::new(580123, 6),
            grid_feed_in: Decimal::new(3, 3),
            device_consumption: Decimal::new(20, 2),
            device_generation: Decimal::new(15, 2),
            storage_consumption: Some(Decimal::new(12, 2)),
            storage_generation: Some(Decimal::new(10, 2)),
            anzulegender_wert: Some(Decimal::new(786, 2)),
            spot_price: Some(Decimal::new(-125, 2)),
        })
        .collect();
    let mut corrected = rows[0];
    corrected.grid_draw += Decimal::ONE;
    for site in ["reference", "meter"] {
        store.put_quarter_hours(site, &rows, receipt).await.unwrap();
        store
            .put_quarter_hour(site, &corrected, receipt + Duration::DAY)
            .await
            .unwrap();
    }
    let declaration = MispelSettings::Abgrenzung {
        basisfall: Basisfall::A4,
        assigned_from: start.date().next_day().unwrap(),
        first_assignment: false,
        operator_consent: true,
    };
    for (at, cents) in [(receipt, 50071), (receipt + Duration::DAY, 50101)] {
        let read = store
            .quarter_hours_as_of("meter", None, None, Some(at))
            .await
            .unwrap();
        assert_eq!(read.len(), 2880);
        let bill: Decimal = read
            .iter()
            .map(|q| q.grid_draw * Decimal::new(3001, 2) - q.grid_feed_in * Decimal::new(786, 2))
            .sum();
        assert_eq!(bill.round(), Decimal::from(cents));
        let export = async |site| {
            histd::export::mispel(
                &store,
                site,
                Some(declaration),
                None,
                2026,
                Some(11),
                Some(at),
            )
            .await
            .unwrap()
        };
        let actual = export("meter").await;
        assert_eq!(actual["complete"], true);
        let reference = export("reference").await;
        let expected = reference["figures"]["abgrenzung"].as_object().unwrap();
        for (key, value) in expected {
            let actual = &actual["figures"]["abgrenzung"][key];
            if let Some(number) = value.as_str().and_then(|s| s.parse::<Decimal>().ok()) {
                assert_eq!(
                    actual.as_str().unwrap().parse::<Decimal>().unwrap(),
                    number,
                    "{key}"
                );
            } else {
                assert_eq!(actual, value, "{key}");
            }
        }
    }
}
