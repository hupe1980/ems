//! The worked examples the Festlegung prints, as tests.
//!
//! Every expected figure here is one the BNetzA wrote down: Anlage 1's
//! Beispielrechnungen 1 and 2 (p. 16), Anlage 2's Beispielsfall with both
//! variants (§ 2.1.6, pp. 14–15), its Tabelle 1 (p. 13) and its Rumpfjahr
//! examples (§ 9.3, pp. 56–57) — BNetzA 618-25-02, Festlegung of 01.10.2026.
//! The published tables round for display, so a table value is compared
//! after rounding to whole kilowatt-hours; everything else is exact.

use ems_core::prelude::Slot;
use ems_grid::mispel::{
    Basisfall, PauschalFall, PauschalPeriod, PauschalPlant, QuarterHour, RuleSet, abgrenzung_month,
    pauschal_period,
};
use rust_decimal::{Decimal, RoundingStrategy};
use time::macros::{date, datetime};

const RULES: RuleSet = RuleSet::Festlegung20261001;

fn kwh(v: i64) -> Decimal {
    Decimal::from(v)
}

fn quarter(i: i64) -> QuarterHour {
    QuarterHour::empty(Slot::containing(datetime!(2027-06-01 00:00:00 +02:00)).offset(i))
}

fn plant(fall: PauschalFall, kwp: i64, storage: i64) -> PauschalPlant {
    PauschalPlant {
        fall,
        solar_kwp: kwh(kwp),
        storage_kwh: kwh(storage),
    }
}

fn whole(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

// ── Anlage 1 ────────────────────────────────────────────────────────────────

/// Evidence for SR-026.
#[test]
fn anlage_1_beispielrechnung_1_the_store_is_charged_from_the_grid_first() {
    // 100 kWh into the store, 130 kWh drawn from the grid, 50 kWh from the roof:
    // the store's 100 kWh came from the grid; the other 30 and the 50 from the
    // roof went to other consumption.
    let q = QuarterHour {
        grid_draw: kwh(130),
        device_consumption: kwh(100),
        anzulegender_wert: kwh(8),
        ..quarter(0)
    };
    assert_eq!(q.simultaneous_grid_charging(), kwh(100), "(1)¼");
    let month = abgrenzung_month(Basisfall::A1, RULES, &[q]).expect("a month");
    assert_eq!(month.grid_charged, kwh(100), "(9)");
    assert_eq!(month.plant_charged, Decimal::ZERO, "(10)");
}

#[test]
fn anlage_1_beispielrechnung_2_the_feed_in_comes_out_of_the_store_first() {
    // 100 kWh out of the store, 50 kWh from the roof, 80 kWh fed in: all 80
    // came out of the store.
    let q = QuarterHour {
        grid_feed_in: kwh(80),
        device_generation: kwh(100),
        anzulegender_wert: kwh(8),
        ..quarter(0)
    };
    assert_eq!(q.simultaneous_device_feed_in(), kwh(80), "(2)¼");
}

// ── Anlage 2 ────────────────────────────────────────────────────────────────

/// A year with `fed_in` kWh at non-negative prices and positive AW, and
/// `drawn` kWh from the grid, spread over whole kilowatt-hour quarter hours.
fn year(fed_in: i64, drawn: i64) -> Vec<QuarterHour> {
    let n = (fed_in.max(drawn) + 9) / 10;
    (0..n)
        .map(|i| QuarterHour {
            grid_feed_in: kwh((fed_in - i * 10).clamp(0, 10)),
            grid_draw: kwh((drawn - i * 10).clamp(0, 10)),
            anzulegender_wert: kwh(8),
            spot_price: kwh(5),
            ..quarter(i)
        })
        .collect()
}

/// Evidence for SR-026.
#[test]
fn anlage_2_beispielsfall_8_kwp_and_10_kwh() {
    let p = pauschal_period(
        plant(PauschalFall::P1, 8, 10),
        RULES,
        PauschalPeriod::Year(2027),
        &year(1, 0),
    )
    .expect("a year");
    assert_eq!(p.support_cap, kwh(4000), "(P1)");
    assert_eq!(p.indifference_band, kwh(320), "(P3)");
    assert_eq!(p.settlement_threshold, kwh(4320), "(P4)");
}

#[test]
fn anlage_2_beispielrechnung_1_low_feed_in_is_all_supportable() {
    let p = pauschal_period(
        plant(PauschalFall::P1, 8, 10),
        RULES,
        PauschalPeriod::Year(2027),
        &year(3000, 2000),
    )
    .expect("a year");
    assert_eq!(p.supported_feed_in, kwh(3000), "all of it förderfähig");
    assert_eq!(
        p.settleable_feed_in,
        Decimal::ZERO,
        "no room for a Saldierung"
    );
}

#[test]
fn anlage_2_beispielrechnung_2_high_feed_in_splits_three_ways() {
    let p = pauschal_period(
        plant(PauschalFall::P1, 8, 10),
        RULES,
        PauschalPeriod::Year(2027),
        &year(6000, 1500),
    )
    .expect("a year");
    assert_eq!(p.supported_feed_in, kwh(4000), "förderfähig up to (P1)");
    assert_eq!(p.above_threshold, kwh(1680), "(P8): above (P4)");
    assert_eq!(
        p.settleable_feed_in,
        kwh(1500),
        "(P10): limited by the draw"
    );
    assert_eq!(
        p.levied_grid_draw,
        Decimal::ZERO,
        "(P11): the levy falls to zero"
    );
}

/// Evidence for SR-026.
#[test]
fn anlage_2_tabelle_1_every_cell() {
    // Pauschalgrenze der Saldierungsfähigkeit (P4) in kWh/a, Basisfall P1.
    let storage = [45, 30, 15, 10, 8, 6, 4];
    let table: [(i64, [i64; 7]); 7] = [
        (1, [501, 502, 503, 505, 506, 508, 513]),
        (4, [2018, 2027, 2053, 2080, 2100, 2133, 2200]),
        (6, [3040, 3060, 3120, 3180, 3225, 3300, 3450]),
        (8, [4071, 4107, 4213, 4320, 4400, 4533, 4800]),
        (10, [5111, 5167, 5333, 5500, 5625, 5833, 6250]),
        (15, [7750, 7875, 8250, 8625, 8906, 9375, 10313]),
        (30, [16000, 16500, 18000, 19500, 20625, 22500, 26250]),
    ];
    for (kwp, row) in table {
        for (sk, printed) in storage.iter().zip(row) {
            let p = pauschal_period(
                plant(PauschalFall::P1, kwp, *sk),
                RULES,
                PauschalPeriod::Year(2027),
                &year(1, 0),
            )
            .expect("a year");
            assert_eq!(
                whole(p.settlement_threshold),
                kwh(printed),
                "{kwp} kWp, {sk} kWh"
            );
        }
    }
}

/// Evidence for SR-026.
#[test]
fn anlage_2_section_9_3_the_two_rumpfjahre_of_a_year() {
    // A second store (5 kWh) is commissioned on 16 May of an ordinary year; the
    // day of the change counts to the Rumpfjahr before it.
    let rumpf = |first, last, storage, at| {
        pauschal_period(
            plant(PauschalFall::P1, 8, storage),
            RULES,
            PauschalPeriod::Rumpfjahr { first, last },
            &[QuarterHour {
                anzulegender_wert: kwh(8),
                ..QuarterHour::empty(Slot::containing(at))
            }],
        )
        .expect("a Rumpfjahr")
    };
    let before = rumpf(
        date!(2027 - 01 - 01),
        date!(2027 - 05 - 16),
        10,
        datetime!(2027-03-01 12:00:00 +01:00),
    );
    let days = before.rumpfjahr.expect("a Rumpfjahr");
    assert_eq!((days.days, days.summer_days), (136, 46));
    assert_eq!(whole(before.support_cap), kwh(1005), "(P1)R");
    assert_eq!(whole(before.indifference_band), kwh(119), "(P3)R");
    assert_eq!(whole(before.settlement_threshold), kwh(1125), "(P4)R");

    let after = rumpf(
        date!(2027 - 05 - 17),
        date!(2027 - 12 - 31),
        15,
        datetime!(2027-07-01 12:00:00 +02:00),
    );
    let days = after.rumpfjahr.expect("a Rumpfjahr");
    assert_eq!((days.days, days.summer_days), (229, 137));
    assert_eq!(whole(after.support_cap), kwh(2995), "(P1)R");
    assert_eq!(whole(after.indifference_band), kwh(134), "(P3)R");
    assert_eq!(whole(after.settlement_threshold), kwh(3128), "(P4)R");
}
