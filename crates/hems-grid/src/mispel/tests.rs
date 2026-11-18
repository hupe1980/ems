use super::*;
use rust_decimal::prelude::FromPrimitive;
use time::macros::date;

const RULES: RuleSet = RuleSet::Festlegung20261001;

fn dec(v: f64) -> Decimal {
    Decimal::from_f64(v).unwrap().round_dp(6)
}

fn slot(i: i64) -> Slot {
    Slot::containing(time::macros::datetime!(2026-11-01 00:00:00 +01:00)).offset(i)
}

/// A quarter hour: (draw, feed-in, device consumption, device generation, AW).
fn q(i: i64, nb: f64, ne: f64, v: f64, e: f64, aw: f64) -> QuarterHour {
    QuarterHour {
        grid_draw: dec(nb),
        grid_feed_in: dec(ne),
        device_consumption: dec(v),
        device_generation: dec(e),
        anzulegender_wert: dec(aw),
        ..QuarterHour::empty(slot(i))
    }
}

// ── (1)/(2): the Speichervorrang ────────────────────────────────────────────

#[test]
fn the_storage_priority_is_a_minimum_per_quarter_hour_and_not_over_the_month() {
    // Per quarter hour only 1 kWh was charged from the grid. Summing first —
    // 4 kWh of draw against 5 kWh of charging — would claim 4, and turn an
    // ordinary day into three kilowatt-hours of arbitrage.
    let month = [q(0, 4.0, 0.0, 1.0, 0.0, 8.0), q(1, 0.0, 0.0, 4.0, 0.0, 8.0)];
    let r = abgrenzung_month(Basisfall::A1, RULES, &month).unwrap();
    assert_eq!(r.grid_charged, dec(1.0));
    assert_eq!(r.plant_charged, dec(4.0));
}

// ── A worked month, checked figure by figure ────────────────────────────────

/// A battery (A1): 100 kWh from the roof and 20 kWh from the grid in, 102 kWh
/// out, 60 of them to the grid at the same time as the store was producing.
fn worked_month() -> Vec<QuarterHour> {
    let mut month = Vec::new();
    for i in 0..20 {
        month.push(q(i, 3.0, 0.0, 1.0, 0.0, 8.0));
    }
    for i in 20..120 {
        month.push(q(i, 0.0, 0.0, 1.0, 0.0, 8.0));
    }
    for i in 120..222 {
        let ne = if i < 180 { 1.0 } else { 0.0 };
        month.push(q(i, 0.0, ne, 0.0, 1.0, 8.0));
    }
    month
}

#[test]
fn a_worked_month_reproduces_every_figure_of_anlage_1() {
    let r = abgrenzung_month(Basisfall::A1, RULES, &worked_month()).unwrap();
    assert_eq!(r.grid_draw, dec(60.0), "(3)");
    assert_eq!(r.grid_feed_in, dec(60.0), "(4)");
    assert_eq!(r.device_consumption, dec(120.0), "(5)");
    assert_eq!(r.device_generation, dec(102.0), "(6)");
    assert_eq!(r.grid_charged, dec(20.0), "(9)");
    assert_eq!(r.plant_charged, dec(100.0), "(10)");
    assert_eq!(r.device_feed_in_base, dec(60.0), "(11)");
    assert_eq!(r.foreign_charge, Decimal::ZERO, "(12)");
    assert_eq!(r.device_feed_in_considered, dec(60.0), "(13)");
    // (14)A1 = 102 / 120 = 0,85 — measured, and landing on the presumed figure.
    assert_eq!(r.efficiency, Some(dec(0.85)), "(14)A1");
    assert_eq!(r.renewable_storage_output, dec(85.0), "(15)");
    // (16) = MAX[60 − 85 ; 0] = 0: nothing the store fed in was grey.
    assert_eq!(r.settleable_feed_in, Decimal::ZERO, "(16)");
    assert_eq!(r.storage_losses, Some(dec(18.0)), "(17)A1");
    assert_eq!(r.privilegeable_losses, Decimal::ZERO, "(19)");
    assert_eq!(r.levied_grid_draw, dec(60.0), "(21)");
    assert_eq!(
        r.supported_direct,
        Decimal::ZERO,
        "(26): all of it via the store"
    );
    assert_eq!(r.supported_feed_in, dec(60.0), "(32)");
}

#[test]
fn a_month_of_grid_arbitrage_produces_a_settleable_quantity() {
    let mut month = Vec::new();
    for i in 0..40 {
        month.push(q(i, 2.0, 0.0, 2.0, 0.0, 8.0));
    }
    for i in 40..108 {
        month.push(q(i, 0.0, 1.0, 0.0, 1.0, 8.0));
    }
    let r = abgrenzung_month(Basisfall::A1, RULES, &month).unwrap();
    assert_eq!(r.grid_charged, dec(80.0));
    assert_eq!(r.plant_charged, Decimal::ZERO);
    assert_eq!(r.renewable_storage_output, Decimal::ZERO, "(15)");
    assert_eq!(r.settleable_feed_in, dec(68.0), "(16)");
    assert_eq!(r.storage_losses, Some(dec(12.0)), "(17)");
    assert_eq!(r.settleable_share, Some(Decimal::ONE), "(18)");
    assert_eq!(r.privilegeable_losses, dec(12.0), "(19)");
    assert_eq!(r.levy_reducing, dec(80.0), "(20)");
    assert_eq!(r.levied_grid_draw, Decimal::ZERO, "(21)");
    assert_eq!(r.levy_relief_share(), Decimal::ONE);
}

#[test]
fn the_levy_reduction_can_never_exceed_the_draw_it_reduces() {
    let mut month = vec![q(0, 1.0, 0.0, 1.0, 0.0, 8.0)];
    for i in 1..50 {
        month.push(q(i, 0.0, 1.0, 0.0, 1.0, 8.0));
    }
    let r = abgrenzung_month(Basisfall::A1, RULES, &month).unwrap();
    assert_eq!(r.levy_reducing, r.grid_draw);
    assert_eq!(r.levied_grid_draw, Decimal::ZERO);
}

#[test]
fn a_negative_price_quarter_hour_carries_no_support() {
    let a = abgrenzung_month(Basisfall::A2, RULES, &[q(0, 0.0, 5.0, 0.0, 0.0, 8.0)]).unwrap();
    let b = abgrenzung_month(Basisfall::A2, RULES, &[q(0, 0.0, 5.0, 0.0, 0.0, 0.0)]).unwrap();
    assert_eq!(a.supported_feed_in, dec(5.0));
    assert_eq!(b.supported_feed_in, Decimal::ZERO);
    assert_eq!(a.grid_feed_in, b.grid_feed_in);
}

#[test]
fn energy_a_car_brought_from_somewhere_else_is_taken_back_out() {
    let month = [q(0, 0.0, 0.0, 2.0, 0.0, 8.0), q(1, 0.0, 6.0, 0.0, 6.0, 8.0)];
    let r = abgrenzung_month(Basisfall::A2, RULES, &month).unwrap();
    assert_eq!(r.foreign_charge, dec(4.0), "(12) = MAX[6 − 2 ; 0]");
    assert_eq!(r.device_feed_in_considered, dec(2.0), "(13) = 6 − 4");
}

#[test]
fn a_charge_point_gets_the_presumed_efficiency_and_no_privilegeable_losses() {
    let month = [q(0, 2.0, 0.0, 2.0, 0.0, 8.0), q(1, 0.0, 1.0, 0.0, 1.0, 8.0)];
    for fall in [Basisfall::A2, Basisfall::A3] {
        let r = abgrenzung_month(fall, RULES, &month).unwrap();
        assert_eq!(r.efficiency, Some(PRESUMED_EFFICIENCY), "(14) in {fall}");
        assert_eq!(r.privilegeable_losses, Decimal::ZERO, "(19) in {fall}");
        assert_eq!(r.storage_losses, None, "(17) is not visible in {fall}");
    }
}

#[test]
fn case_a4_measures_the_storage_losses_on_their_own_meter() {
    let month: Vec<QuarterHour> = (0..2)
        .map(|i| QuarterHour {
            storage_consumption: Some(dec(if i == 0 { 2.0 } else { 0.0 })),
            storage_generation: Some(dec(if i == 0 { 0.0 } else { 1.7 })),
            ..q(
                i,
                if i == 0 { 4.0 } else { 0.0 },
                if i == 0 { 0.0 } else { 4.0 },
                if i == 0 { 4.0 } else { 0.0 },
                if i == 0 { 0.0 } else { 4.0 },
                8.0,
            )
        })
        .collect();
    let a4 = abgrenzung_month(Basisfall::A4, RULES, &month).unwrap();
    assert_eq!(a4.storage_losses, Some(dec(0.3)), "(17)A4 = 2,0 − 1,7");
    assert!(a4.privilegeable_losses > Decimal::ZERO);
    let a3 = abgrenzung_month(Basisfall::A3, RULES, &month).unwrap();
    assert_eq!(a3.privilegeable_losses, Decimal::ZERO, "(19)A2,A3");
}

#[test]
fn case_a4_refuses_to_guess_at_a_meter_it_does_not_have() {
    assert_eq!(
        abgrenzung_month(Basisfall::A4, RULES, &[q(0, 1.0, 0.0, 1.0, 0.0, 8.0)]),
        Err(MispelError::MissingStorageMeter)
    );
}

#[test]
fn a_month_the_store_took_nothing_is_a_figure_and_not_an_error() {
    // (14)A1 = (6)/(5) is undefined when (5) = 0, and (15) = (14)•(10) is zero
    // because (10) is. The copy hems held refused the whole month; the
    // Festlegung gives it numbers.
    let r = abgrenzung_month(Basisfall::A1, RULES, &[q(0, 3.0, 1.0, 0.0, 0.0, 8.0)]).unwrap();
    assert_eq!(r.efficiency, None, "(14)A1 undefined");
    assert_eq!(r.renewable_storage_output, Decimal::ZERO, "(15)");
    assert_eq!(r.levied_grid_draw, dec(3.0), "(21)");
    assert_eq!(
        r.supported_feed_in,
        dec(1.0),
        "(32): the roof fed in directly"
    );
}

#[test]
fn a_quarter_hour_of_another_month_is_refused() {
    let mut month = worked_month();
    month.push(QuarterHour {
        slot: Slot::containing(time::macros::datetime!(2026-12-01 00:00:00 +01:00)),
        ..month[0]
    });
    assert!(matches!(
        abgrenzung_month(Basisfall::A1, RULES, &month),
        Err(MispelError::OutsidePeriod(_))
    ));
}

#[test]
fn an_empty_month_is_a_missing_measurement_and_not_a_zero() {
    assert_eq!(
        abgrenzung_month(Basisfall::A1, RULES, &[]),
        Err(MispelError::NoValues)
    );
    assert_eq!(
        pauschal_period(
            plant(PauschalFall::P1, 9.8, 10.0),
            RULES,
            PauschalPeriod::Year(2026),
            &[]
        ),
        Err(MispelError::NoValues)
    );
}

#[test]
fn a_meter_that_counts_backwards_is_refused_rather_than_clamped() {
    let mut month = vec![q(0, 1.0, 0.0, 1.0, 0.0, 8.0)];
    month[0].grid_draw = dec(-1.0);
    assert!(matches!(
        abgrenzung_month(Basisfall::A1, RULES, &month),
        Err(MispelError::NegativeValue { field: "Z1NB", .. })
    ));
}

// ── The Pauschaloption ──────────────────────────────────────────────────────

fn plant(fall: PauschalFall, kwp: f64, kwh: f64) -> PauschalPlant {
    PauschalPlant {
        fall,
        solar_kwp: dec(kwp),
        storage_kwh: dec(kwh),
    }
}

fn year_of(p: PauschalPlant) -> Pauschal {
    pauschal_period(
        p,
        RULES,
        PauschalPeriod::Year(2026),
        &[q(0, 0.0, 0.0, 0.0, 0.0, 8.0)],
    )
    .unwrap()
}

#[test]
fn the_pauschal_thresholds_come_out_of_the_size_of_the_installation() {
    // 9,8 kWp and 10 kWh: (P1) = 4 900, (P2)P1 = 0,1 × 9,8 / 10 = 0,098, so
    // (P3) = 480,2 and (P4) = 5 380,2.
    let r = year_of(plant(PauschalFall::P1, 9.8, 10.0));
    assert_eq!(r.support_cap, dec(4900.0), "(P1)");
    assert_eq!(r.indifference_band, dec(480.2), "(P3)");
    assert_eq!(r.settlement_threshold, dec(5380.2), "(P4)");
}

#[test]
fn a_smaller_store_widens_the_indifference_band() {
    let small = year_of(plant(PauschalFall::P1, 10.0, 5.0)).indifference_band;
    let large = year_of(plant(PauschalFall::P1, 10.0, 20.0)).indifference_band;
    assert!(small > large, "{small} should exceed {large}");
}

/// Evidence for SR-026.
#[test]
fn a_charge_point_takes_a_fifth_and_both_take_the_smaller_of_the_two() {
    // The adopted (P2)P2 is 0,2 — the Arbeitsstand said 0,5.
    assert_eq!(
        year_of(plant(PauschalFall::P2, 10.0, 0.0)).indifference_band,
        dec(1000.0)
    );
    // P3 takes MIN[(P2)P1 ; (P2)P2] = MIN[0,05 ; 0,2].
    assert_eq!(
        year_of(plant(PauschalFall::P3, 10.0, 20.0)).indifference_band,
        dec(250.0)
    );
}

#[test]
fn a_storage_case_without_a_capacity_is_refused() {
    assert_eq!(
        pauschal_period(
            plant(PauschalFall::P1, 8.0, 0.0),
            RULES,
            PauschalPeriod::Year(2026),
            &[q(0, 0.0, 0.0, 0.0, 0.0, 8.0)]
        ),
        Err(MispelError::NoStorageCapacity(PauschalFall::P1))
    );
}

#[test]
fn only_feed_in_above_the_threshold_and_at_non_negative_prices_is_settleable() {
    // (P1) = 500, (P2)P1 = 0,01, (P3) = 5, (P4) = 505.
    let mut year: Vec<QuarterHour> = (0..600)
        .map(|i| QuarterHour {
            spot_price: dec(5.0),
            ..q(i, 1.0, 1.0, 0.0, 0.0, 8.0)
        })
        .collect();
    for h in year.iter_mut().take(50) {
        h.spot_price = dec(-2.0);
    }
    let r = pauschal_period(
        plant(PauschalFall::P1, 1.0, 10.0),
        RULES,
        PauschalPeriod::Year(2026),
        &year,
    )
    .unwrap();
    assert_eq!(r.feed_in_at_non_negative_prices, dec(550.0), "(P7)");
    assert_eq!(r.above_threshold, dec(45.0), "(P8) = 550 − 505");
    assert_eq!(r.grid_draw, dec(600.0), "(P9)");
    assert_eq!(r.settleable_feed_in, dec(45.0), "(P10)");
    assert_eq!(r.levied_grid_draw, dec(555.0), "(P11)");
    assert_eq!(r.supported_feed_in, dec(500.0), "(P16), capped at (P1)");
}

#[test]
fn the_pauschal_option_refuses_a_roof_it_is_not_open_to() {
    assert_eq!(
        pauschal_period(
            plant(PauschalFall::P1, 45.0, 10.0),
            RULES,
            PauschalPeriod::Year(2026),
            &[q(0, 0.0, 0.0, 0.0, 0.0, 8.0)]
        ),
        Err(MispelError::TooLargeForPauschal(dec(45.0)))
    );
}

// ── The rule set and the governed window ────────────────────────────────────

#[test]
fn the_rules_take_effect_two_weeks_after_the_amtsblatt() {
    assert!(!RULES.applies_on(date!(2026 - 10 - 20)));
    assert!(RULES.applies_on(date!(2026 - 10 - 21)));
    assert_eq!(RULES.consent_required_until(), Some(date!(2027 - 09 - 30)));
}

#[test]
fn the_pauschaloption_waits_for_the_month_after_state_aid() {
    assert_eq!(RULES.pauschal_applicable_from(None), None);
    assert_eq!(
        RULES.pauschal_applicable_from(Some(date!(2027 - 02 - 10))),
        Some(date!(2027 - 03 - 01))
    );
    assert_eq!(
        RULES.pauschal_applicable_from(Some(date!(2027 - 12 - 31))),
        Some(date!(2028 - 01 - 01))
    );
}

fn conditions(from: time::Date, first: bool, consent: bool, aid: Option<time::Date>) -> Conditions {
    Conditions {
        assignment: Assignment { from, first },
        operator_consent: consent,
        state_aid_approved_on: aid,
    }
}

/// Evidence for SR-028.
#[test]
fn an_existing_plant_switches_on_the_first_of_a_month_and_only_after_the_rules() {
    assert_eq!(
        Assignment {
            from: date!(2026 - 11 - 01),
            first: false
        }
        .validate(RULES),
        Ok(())
    );
    assert!(matches!(
        Assignment {
            from: date!(2026 - 11 - 12),
            first: false
        }
        .validate(RULES),
        Err(Refusal::MidMonthSwitch { .. })
    ));
    assert_eq!(
        Assignment {
            from: date!(2026 - 11 - 12),
            first: true
        }
        .validate(RULES),
        Ok(())
    );
    assert!(matches!(
        Assignment {
            from: date!(2026 - 10 - 01),
            first: false
        }
        .validate(RULES),
        Err(Refusal::AssignedBeforeTheRules { .. })
    ));
}

/// Evidence for SR-027.
#[test]
fn october_2026_is_not_governed_for_a_switch_and_november_is_whole() {
    let c = conditions(date!(2026 - 11 - 01), false, true, None);
    assert!(matches!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2026 - 10 - 01),
            date!(2026 - 10 - 31)
        ),
        Err(Refusal::BeforeTheAssignment { .. })
    ));
    assert_eq!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2026 - 11 - 01),
            date!(2026 - 11 - 30)
        ),
        Ok(Governed {
            first: date!(2026 - 11 - 01),
            last: date!(2026 - 11 - 30),
            rumpf: false
        })
    );
}

#[test]
fn a_first_assignment_mid_month_is_a_rumpfmonat() {
    let c = conditions(date!(2026 - 11 - 12), true, true, None);
    assert_eq!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2026 - 11 - 01),
            date!(2026 - 11 - 30)
        ),
        Ok(Governed {
            first: date!(2026 - 11 - 12),
            last: date!(2026 - 11 - 30),
            rumpf: true
        })
    );
}

/// Evidence for SR-027.
#[test]
fn without_consent_nothing_before_october_2027_is_governed() {
    let c = conditions(date!(2026 - 11 - 01), false, false, None);
    assert_eq!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2027 - 09 - 01),
            date!(2027 - 09 - 30)
        ),
        Err(Refusal::NoConsent {
            until: date!(2027 - 09 - 30)
        })
    );
    assert_eq!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2027 - 10 - 01),
            date!(2027 - 10 - 31)
        ),
        Ok(Governed {
            first: date!(2027 - 10 - 01),
            last: date!(2027 - 10 - 31),
            rumpf: false
        })
    );
}

/// Evidence for SR-027.
#[test]
fn the_pauschal_year_after_state_aid_is_a_rumpfjahr() {
    let none = conditions(date!(2026 - 11 - 01), false, true, None);
    assert_eq!(
        governed(
            RULES,
            MispelOption::Pauschal,
            none,
            date!(2027 - 01 - 01),
            date!(2027 - 12 - 31)
        ),
        Err(Refusal::NoStateAid)
    );
    let approved = conditions(
        date!(2026 - 11 - 01),
        false,
        true,
        Some(date!(2027 - 02 - 10)),
    );
    assert_eq!(
        governed(
            RULES,
            MispelOption::Pauschal,
            approved,
            date!(2027 - 01 - 01),
            date!(2027 - 12 - 31)
        ),
        Ok(Governed {
            first: date!(2027 - 03 - 01),
            last: date!(2027 - 12 - 31),
            rumpf: true
        })
    );
    assert!(matches!(
        governed(
            RULES,
            MispelOption::Pauschal,
            approved,
            date!(2026 - 01 - 01),
            date!(2026 - 12 - 31)
        ),
        Err(Refusal::BeforeStateAid { .. })
    ));
}

#[test]
fn a_period_before_the_rules_is_refused_naming_the_effect_date() {
    let c = conditions(date!(2026 - 11 - 01), false, true, None);
    assert_eq!(
        governed(
            RULES,
            MispelOption::Abgrenzung,
            c,
            date!(2026 - 09 - 01),
            date!(2026 - 09 - 30)
        ),
        Err(Refusal::BeforeTheRules {
            effective_from: date!(2026 - 10 - 21)
        })
    );
}
