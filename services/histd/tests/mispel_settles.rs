//! The MiSpeL settlement, over registers the way a box actually writes them.
//!
//! A household's levy privileges (§ 21 EnFG) and its EEG support depend on
//! this document from 21.10.2026, when the Festlegung of 01.10.2026 took
//! effect. What these tests hold is the part a unit test cannot: that the
//! registers `histd` keeps, the window the rules govern for *this* site, and
//! `metering`'s arithmetic meet in one document that says what it rests on.

use ems_grid::mispel::{Basisfall, PauschalFall, QuarterHour};
use histd::Store;
use histd::config::MispelSettings;
use histd::export::mispel;
use rust_decimal::Decimal;
use time::macros::date;

/// A month of registers as the control loop writes them: a household drawing
/// from the grid, feeding a little back, and cycling its store.
///
/// The four register names are the Festlegung's own — `Z1NB¼`, `Z1NE¼`,
/// `Z2V¼`, `Z2E¼` — and the values are deliberately all different, so a
/// settlement that read one for another fails here rather than reconciling.
async fn month_of_registers(store: &Store, site: &str, year: i32, month: u8, quarters: usize) {
    let first = time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), 1)
        .expect("a real month");
    let start = ems_core::calendar::day_start_utc(first);
    let now = start;
    for i in 0..quarters {
        let slot =
            ems_core::prelude::Slot::containing(start + time::Duration::minutes(15 * i as i64));
        store
            .put_quarter_hour(
                site,
                &QuarterHour {
                    // 0,580 kWh drawn, 0,003 fed back, and a store that took
                    // 0,200 and gave 0,150 — a perfectly ordinary quarter hour.
                    grid_draw: Decimal::new(580, 3),
                    grid_feed_in: Decimal::new(3, 3),
                    device_consumption: Decimal::new(200, 3),
                    device_generation: Decimal::new(150, 3),
                    anzulegender_wert: Decimal::new(786, 2),
                    spot_price: Decimal::new(1250, 2),
                    ..QuarterHour::empty(slot)
                },
                now,
            )
            .await
            .expect("a register the box wrote");
    }
}

/// The same month, with the storage system's **own** meter beside `Z2`.
///
/// This is what a box on Basisfall A4 writes: `Z3V¼`/`Z3E¼` are a subset of
/// `Z2V¼`/`Z2E¼` — the store's half of what the store and the charge point did
/// together — and the difference between them is what the charge point did.
async fn month_of_registers_with_a_metered_store(
    store: &Store,
    site: &str,
    year: i32,
    month: u8,
    quarters: usize,
) {
    let first = time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), 1)
        .expect("a real month");
    let start = ems_core::calendar::day_start_utc(first);
    for i in 0..quarters {
        let slot =
            ems_core::prelude::Slot::containing(start + time::Duration::minutes(15 * i as i64));
        store
            .put_quarter_hour(
                site,
                &QuarterHour {
                    grid_draw: Decimal::new(580, 3),
                    grid_feed_in: Decimal::new(3, 3),
                    device_consumption: Decimal::new(200, 3),
                    device_generation: Decimal::new(150, 3),
                    // Of that pair, the store's own half — the charge point took
                    // the rest.
                    storage_consumption: Some(Decimal::new(120, 3)),
                    storage_generation: Some(Decimal::new(100, 3)),
                    anzulegender_wert: Decimal::new(786, 2),
                    spot_price: Decimal::new(1250, 2),
                    ..QuarterHour::empty(slot)
                },
                start,
            )
            .await
            .expect("a register the box wrote");
    }
}

/// An Abgrenzung site that switched an existing plant on 1 November 2026 —
/// the first day the text allows — with its operators' consent.
fn abgrenzung(basisfall: Basisfall) -> MispelSettings {
    MispelSettings::Abgrenzung {
        basisfall,
        assigned_from: date!(2026 - 11 - 01),
        first_assignment: false,
        operator_consent: true,
    }
}

fn pauschal(assigned_from: time::Date) -> MispelSettings {
    MispelSettings::Pauschal {
        fall: PauschalFall::P1,
        solar_kwp: 9.8,
        storage_kwh: 10.0,
        assigned_from,
        first_assignment: false,
        operator_consent: true,
    }
}

/// October 2027: 31 days and the long day of the clock change, so 2 980 quarter
/// hours rather than 2 976 — and after the consent deadline.
const OCTOBER_2027: usize = 31 * 96 + 4;
/// November 2026: thirty ordinary days.
const NOVEMBER_2026: usize = 30 * 96;

/// Evidence for SR-029.
#[tokio::test]
async fn a_full_month_settles_and_says_what_it_rests_on() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2027, 10, OCTOBER_2027).await;

    let doc = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A1)),
        None,
        2027,
        Some(10),
        None,
    )
    .await
    .expect("a month of registers settles");

    assert_eq!(doc["settled"], true);
    assert_eq!(
        doc["quarter_hours_expected"], OCTOBER_2027,
        "the long October day has a hundred quarter hours, not ninety-six"
    );
    assert_eq!(doc["complete"], true);
    assert_eq!(doc["rule_set"], "BNetzA 618-25-02, Festlegung 01.10.2026");
    assert_eq!(doc["effective_from"], "2026-10-21");
    assert_eq!(doc["window"]["first"], "2027-10-01");
    assert_eq!(doc["window"]["last"], "2027-10-31");
    assert_eq!(doc["window"]["rumpf"], serde_json::Value::Null);
    assert_eq!(doc["conditions"]["assigned_from"], "2026-11-01");
    assert_eq!(doc["conditions"]["operator_consent"], true);
    assert_eq!(doc["conditions"]["consent_required_until"], "2027-09-30");

    let a = &doc["figures"]["abgrenzung"];
    assert!(
        a["grid_draw"].as_str().is_some(),
        "the numbered figures: {a}"
    );
    let relief: f64 = doc["figures"]["levy_relief_share"]
        .as_str()
        .expect("the one number a household wants")
        .parse()
        .unwrap();
    assert!(
        (0.0..=1.0).contains(&relief),
        "a share of the grid draw: {relief}"
    );
}

#[tokio::test]
async fn basisfall_a4_settles_from_the_storage_system_s_own_meter() {
    // `[MiSpeL A1 (17)A4]` charges the conversion losses to the store, which A3
    // cannot see and therefore charges to the household.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers_with_a_metered_store(&store, "haus-a4", 2027, 10, OCTOBER_2027).await;

    let doc = mispel(
        &store,
        "haus-a4",
        Some(abgrenzung(Basisfall::A4)),
        None,
        2027,
        Some(10),
        None,
    )
    .await
    .expect("A4 settles once the store's own registers reach the fleet");
    assert_eq!(doc["complete"], true);
    assert_eq!(
        doc["figures"]["abgrenzung"]["storage_losses"], "59.600",
        "2 980 quarter hours of 0,020 kWh — a null here would mean the separate \
         meter never arrived"
    );
}

#[tokio::test]
async fn a_settlement_can_be_reproduced_from_the_registers_it_was_computed_from() {
    // A Nachweis is settled and handed over; weeks later a substitute value is
    // replaced by a real reading. *What did we hand over* and *what does the
    // correction change* are both answerable because the registers are
    // versioned.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 11, NOVEMBER_2026).await;
    let option = abgrenzung(Basisfall::A1);

    let first = mispel(&store, "haus-1", Some(option), None, 2026, Some(11), None)
        .await
        .expect("the month settles");
    // The instant the document was produced at: the day after the period, not
    // the wall clock.
    let settled_at = ems_core::calendar::day_start_utc(date!(2026 - 12 - 01));
    let before = first["figures"]["abgrenzung"]["grid_draw"]
        .as_str()
        .unwrap()
        .to_owned();

    let slot = ems_core::prelude::Slot::containing(ems_core::calendar::day_start_utc(date!(
        2026 - 11 - 01
    )));
    store
        .put_quarter_hour(
            "haus-1",
            &QuarterHour {
                grid_draw: Decimal::new(1_160, 3),
                grid_feed_in: Decimal::new(3, 3),
                device_consumption: Decimal::new(200, 3),
                device_generation: Decimal::new(150, 3),
                anzulegender_wert: Decimal::new(786, 2),
                spot_price: Decimal::new(1250, 2),
                ..QuarterHour::empty(slot)
            },
            settled_at + time::Duration::days(14),
        )
        .await
        .expect("a restated register");

    let corrected = mispel(&store, "haus-1", Some(option), None, 2026, Some(11), None)
        .await
        .expect("the corrected month settles");
    assert_eq!(corrected["quarter_hours_present"], NOVEMBER_2026);
    assert_ne!(
        corrected["figures"]["abgrenzung"]["grid_draw"]
            .as_str()
            .unwrap(),
        before
    );

    let reproduced = mispel(
        &store,
        "haus-1",
        Some(option),
        None,
        2026,
        Some(11),
        Some(settled_at),
    )
    .await
    .expect("the settlement as it stood");
    assert_eq!(
        reproduced["figures"]["abgrenzung"]["grid_draw"]
            .as_str()
            .unwrap(),
        before
    );
    assert_eq!(reproduced["as_of"], settled_at.unix_timestamp());
}

#[tokio::test]
async fn basisfall_a4_is_refused_where_the_store_is_not_separately_metered() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 11, NOVEMBER_2026).await;
    let error = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A4)),
        None,
        2026,
        Some(11),
        None,
    )
    .await
    .expect_err("A4 without Z3 is not a settlement");
    assert!(
        error.to_string().contains("Z3V"),
        "and it names the registers it wanted: {error}"
    );
}

#[tokio::test]
async fn a_month_with_gaps_is_settled_and_is_not_called_complete() {
    // A quarter hour the box could not price gets no register, so a month can
    // have gaps; a settlement over part of it must not look like a whole one.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 11, 2_000).await;
    let doc = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A1)),
        None,
        2026,
        Some(11),
        None,
    )
    .await
    .expect("a partial month still computes");
    assert_eq!(doc["quarter_hours_present"], 2_000);
    assert_eq!(doc["quarter_hours_expected"], NOVEMBER_2026);
    assert_eq!(doc["complete"], false);
}

#[tokio::test]
async fn the_option_decides_which_period_may_be_asked_for() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2027, 1, 96).await;
    let aid = Some(date!(2026 - 12 - 15));

    assert!(
        mispel(
            &store,
            "haus-1",
            Some(abgrenzung(Basisfall::A1)),
            None,
            2027,
            None,
            None
        )
        .await
        .is_err(),
        "a month is required"
    );
    let p = pauschal(date!(2026 - 11 - 01));
    assert!(
        mispel(&store, "haus-1", Some(p), aid, 2027, Some(1), None)
            .await
            .is_err(),
        "a month must be omitted"
    );
    let year = mispel(&store, "haus-1", Some(p), aid, 2027, None, None)
        .await
        .expect("the Pauschaloption settles a year");
    assert_eq!(year["figures"]["option"], "pauschal");
    assert_eq!(
        year["window"]["rumpf"],
        serde_json::Value::Null,
        "approved in 2026: all of 2027"
    );
    assert_eq!(year["quarter_hours_expected"], 365 * 96);
}

#[tokio::test]
async fn a_period_the_festlegung_never_reached_is_refused() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 3, 96).await;
    let error = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A1)),
        None,
        2026,
        Some(3),
        None,
    )
    .await
    .expect_err("March 2026 is before the rules");
    assert!(
        error.to_string().contains("2026-10-21"),
        "and it says when they start: {error}"
    );
}

/// Evidence for SR-027.
#[tokio::test]
async fn an_existing_plant_settles_from_the_month_after_the_rules_and_not_october() {
    // The Festlegung took effect on 21.10.2026, but an installation already
    // assigned to a Veräußerungsform switches only on the first of a month:
    // for an existing roof the first Abgrenzung month is November.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 10, 31 * 96 + 4).await;
    let error = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A1)),
        None,
        2026,
        Some(10),
        None,
    )
    .await
    .expect_err("October is before the assignment");
    assert!(error.to_string().contains("2026-11-01"), "{error}");
}

/// Evidence for SR-029.
#[tokio::test]
async fn a_first_assignment_mid_month_settles_a_rumpfmonat() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-new", 2026, 11, NOVEMBER_2026).await;
    let new_plant = MispelSettings::Abgrenzung {
        basisfall: Basisfall::A1,
        assigned_from: date!(2026 - 11 - 12),
        first_assignment: true,
        operator_consent: true,
    };
    let doc = mispel(
        &store,
        "haus-new",
        Some(new_plant),
        None,
        2026,
        Some(11),
        None,
    )
    .await
    .expect("the Rumpfmonat settles");
    assert_eq!(doc["window"]["first"], "2026-11-12");
    assert_eq!(doc["window"]["rumpf"], "rumpfmonat");
    assert_eq!(doc["conditions"]["first_assignment"], true);
    assert_eq!(doc["quarter_hours_expected"], 19 * 96, "12 to 30 November");
    assert_eq!(
        doc["quarter_hours_present"],
        19 * 96,
        "only the governed days are read"
    );
}

/// Evidence for SR-027.
#[tokio::test]
async fn without_the_operators_consent_nothing_before_october_2027_settles() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 11, NOVEMBER_2026).await;
    month_of_registers(&store, "haus-1", 2027, 10, OCTOBER_2027).await;
    let unconsented = MispelSettings::Abgrenzung {
        basisfall: Basisfall::A1,
        assigned_from: date!(2026 - 11 - 01),
        first_assignment: false,
        operator_consent: false,
    };
    let error = mispel(
        &store,
        "haus-1",
        Some(unconsented),
        None,
        2026,
        Some(11),
        None,
    )
    .await
    .expect_err("Tenorziffer 9 a)");
    assert!(error.to_string().contains("9 a"), "{error}");
    let october = mispel(
        &store,
        "haus-1",
        Some(unconsented),
        None,
        2027,
        Some(10),
        None,
    )
    .await
    .expect("from October 2027 consent is no longer a condition");
    assert_eq!(october["conditions"]["operator_consent"], false);
}

/// Evidence for SR-027 and SR-029.
#[tokio::test]
async fn the_pauschaloption_waits_for_state_aid_and_then_settles_a_rumpfjahr() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-p", 2027, 3, 96).await;
    let p = pauschal(date!(2026 - 11 - 01));

    let error = mispel(&store, "haus-p", Some(p), None, 2027, None, None)
        .await
        .expect_err("no state aid, no Pauschaloption");
    assert!(error.to_string().contains("9 b"), "{error}");

    let doc = mispel(
        &store,
        "haus-p",
        Some(p),
        Some(date!(2027 - 02 - 10)),
        2027,
        None,
        None,
    )
    .await
    .expect("approved in February: from March");
    assert_eq!(doc["window"]["first"], "2027-03-01");
    assert_eq!(doc["window"]["rumpf"], "rumpfjahr");
    assert_eq!(doc["conditions"]["pauschal_applicable_from"], "2027-03-01");
    let rumpf = &doc["figures"]["pauschal"]["rumpfjahr"];
    assert_eq!(rumpf["days"], 306, "1 March to 31 December 2027");
    assert_eq!(rumpf["summer_days"], 183, "every day April to September");
}

#[tokio::test]
async fn a_site_that_declared_nothing_is_refused_rather_than_guessed_at() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2026, 11, 96).await;
    let err = mispel(&store, "haus-1", None, None, 2026, Some(11), None)
        .await
        .expect_err("no declaration, no settlement");
    assert!(
        matches!(err, histd::export::MispelExportError::Undeclared { .. }),
        "{err}"
    );
}

/// Exclusivity settles nothing and is **checked** rather than taken on trust.
///
/// A store that is never charged from the grid has no grey energy to separate,
/// so there is nothing to settle — but "nothing to settle" is not the same as
/// "nothing to say". The claim is a statement about the registers, `(1)¼ =
/// MIN[Z1NB¼ ; Z2V¼]` is what measures it, and the registers are right there.
/// Returning the declaration alone made the one option whose Nachweis is a
/// promise the one option nothing verified.
#[tokio::test]
async fn exclusivity_settles_nothing_and_proves_it_from_the_registers() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    // A month whose every quarter hour draws 0,580 kWh from the grid *and*
    // puts 0,200 into the store: the fixture is an ordinary household, and for
    // this option it is a month of broken claim.
    month_of_registers(&store, "haus-1", 2026, 10, 31 * 96 + 4).await;
    let doc = mispel(
        &store,
        "haus-1",
        Some(MispelSettings::Ausschliesslichkeit),
        None,
        2026,
        Some(10),
        None,
    )
    .await
    .expect("exclusivity always answers");
    assert_eq!(doc["settled"], false);
    assert_eq!(doc["option"], "ausschliesslichkeit");
    assert_eq!(
        doc["held"], false,
        "0,200 kWh into the store every quarter hour, under grid draw"
    );
    assert!(
        !doc["breaches"].as_array().expect("a list").is_empty(),
        "a household needs the quarter hour, not only a total"
    );
    assert!(
        doc["notice"]
            .as_str()
            .is_some_and(|s| s.contains("does not cover this period"))
    );
}

/// …and a household that kept the promise is told so, with the figure.
#[tokio::test]
async fn exclusivity_held_reports_zero_rather_than_silence() {
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    let first =
        time::Date::from_calendar_date(2026, time::Month::October, 1).expect("a real month");
    let start = ems_core::calendar::day_start_utc(first);
    for i in 0..(31 * 96 + 4) {
        let slot = ems_core::prelude::Slot::containing(start + time::Duration::minutes(15 * i));
        store
            .put_quarter_hour(
                "haus-2",
                &QuarterHour {
                    // The meter runs, and the store is idle while it does.
                    grid_draw: Decimal::new(580, 3),
                    grid_feed_in: Decimal::ZERO,
                    device_consumption: Decimal::ZERO,
                    device_generation: Decimal::new(150, 3),
                    anzulegender_wert: Decimal::new(786, 2),
                    spot_price: Decimal::new(1250, 2),
                    ..QuarterHour::empty(slot)
                },
                start,
            )
            .await
            .expect("a register the box wrote");
    }
    let doc = mispel(
        &store,
        "haus-2",
        Some(MispelSettings::Ausschliesslichkeit),
        None,
        2026,
        Some(10),
        None,
    )
    .await
    .expect("exclusivity always answers");
    assert_eq!(doc["held"], true);
    assert_eq!(doc["gleichzeitiger_netzbezug_kwh"], "0");
    assert_eq!(doc["complete"], true);
    assert!(doc["breaches"].as_array().expect("a list").is_empty());
}

#[tokio::test]
async fn a_settlement_covers_only_the_month_it_names() {
    // The window is half-open and comes from the Berlin calendar, so the
    // register at 00:00 on the first of the next month belongs to that month.
    let fixture = ems_service::testdb::Postgres::start(histd::store::MIGRATIONS).await;
    let store = Store::new(fixture.db.clone());
    month_of_registers(&store, "haus-1", 2027, 10, OCTOBER_2027).await;
    month_of_registers(&store, "haus-1", 2027, 11, 96).await;
    let october = mispel(
        &store,
        "haus-1",
        Some(abgrenzung(Basisfall::A1)),
        None,
        2027,
        Some(10),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        october["quarter_hours_present"], OCTOBER_2027,
        "November's registers are November's"
    );
}
