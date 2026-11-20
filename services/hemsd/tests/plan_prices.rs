//! What a plan's two costs cover, held against the day that followed, and a
//! hot-water goal through a whole day.
//!
//! The plan that opens a reference day covers exactly that day's ninety-six
//! slots, so its expected cost and its baseline ask of a forecast what the day
//! report asks of a meter. They were never held together, and in 0.3 the
//! baseline was €93 for a day whose unmanaged house cost €22.

use hems_core::prelude::Asset;
use hemsd::home::{TankGoal, Verdict, judge_tank};
use hemsd::scenario::{Recording, run_recorded};
use hemsd::{HouseholdConfig, Scenario};

/// The opening plan of `scenario` against the day's own report.
fn opening_plan_against_its_day(scenario: &Scenario, bill_tolerance_eur: f64) {
    let (day, recording) = run_recorded(scenario).expect("the day runs");
    let plan = recording.plans.first().expect("a plan opens the day");
    let expected = plan.expected_cost.expect("a plan prices itself");
    let baseline = plan.baseline_cost.expect("and the house without it");

    // The same house, held in the same band: an unmanaged thermostat the heat
    // pump can serve is not paying for discomfort the plan avoids.
    assert!(
        baseline.discomfort_eur < 1.0,
        "the plan's baseline leaves its household {:.2} € uncomfortable",
        baseline.discomfort_eur
    );
    // The baseline the plan prices is the baseline the day measures, within
    // what a forecast and a quarter-hour model can differ by.
    let measured = day.baseline.total();
    assert!(
        (baseline.total() - measured).abs() <= 0.35 * measured.abs().max(4.0),
        "the plan prices the unmanaged house at {:.2} € and the day measured {measured:.2} €",
        baseline.total()
    );
    // And the bill the plan expects is the bill the day got. The difference
    // has one name — the plan saw a forecast and the day saw its own weather —
    // which is why the tolerance is the scenario's.
    let billed = day.cost.billed_eur();
    assert!(
        (expected.billed_eur() - billed).abs() <= bill_tolerance_eur,
        "the plan expected a bill of {:.2} € and the day's was {billed:.2} €",
        expected.billed_eur()
    );
}

#[test]
fn the_january_plan_prices_the_january_day() {
    opening_plan_against_its_day(
        &Scenario::winter_with_grid_event(HouseholdConfig::default()),
        2.0,
    );
}

#[test]
fn the_july_plan_prices_the_july_day() {
    // A wider band in euros on a day whose bill is a few of them: the roof is
    // the whole of the bill in July, and the forecast is deliberately not the
    // day's own sky.
    opening_plan_against_its_day(&Scenario::summer_surplus(HouseholdConfig::default()), 3.5);
}

/// Heat in the tank at `minute` of the recorded day, kWh above its floor.
fn tank_at(recording: &Recording, minute: usize) -> f64 {
    recording.frames[minute]
        .tank
        .expect("the reference tank")
        .stored_now
        .kwh()
}

#[test]
fn hot_water_asked_for_by_six_is_there_at_six() {
    let base = Scenario::winter_evening_deadline(HouseholdConfig::default());
    let six = 18 * 60;
    let asked = |target_c| Scenario {
        tank_goal: Some(TankGoal {
            target_c,
            by_minute: 18 * 60,
        }),
        ..base.clone()
    };

    let (_, free) = run_recorded(&base).expect("the day runs");
    let tank = free
        .site
        .assets
        .iter()
        .find_map(|a| match a {
            Asset::Dhw(t) => Some(t.clone()),
            _ => None,
        })
        .expect("the reference tank");
    let noon = &free.frames[12 * 60];
    let at_noon = noon.status.at.expect("a clock");
    let judged = |target_c| {
        judge_tank(
            &tank,
            &noon.tank.expect("the tank"),
            TankGoal {
                target_c,
                by_minute: 18 * 60,
            },
            at_noon,
        )
    };

    // 50 °C by 18:00: possible, and delivered.
    assert_ne!(judged(50.0).verdict, Verdict::Impossible);
    let (_, fifty) = run_recorded(&asked(50.0)).expect("the day runs");
    assert!(
        tank_at(&fifty, six) >= tank.stored_heat(50.0).kwh() - 0.05,
        "50 °C by six: {:.2} kWh in the tank, {:.2} asked",
        tank_at(&fifty, six),
        tank.stored_heat(50.0).kwh()
    );

    // 58 °C is a goal the free plan does not meet by itself — it lets the tank
    // coast into the evening — so this is the case that shows the plan obeys.
    let wanted = tank.stored_heat(58.0).kwh();
    assert!(
        tank_at(&free, six) < wanted - 0.2,
        "the day chosen must be one where the goal changes something: {:.2} kWh without it",
        tank_at(&free, six)
    );
    let (_, hot) = run_recorded(&asked(58.0)).expect("the day runs");
    assert!(
        tank_at(&hot, six) >= wanted - 0.05,
        "58 °C by six: {:.2} kWh in the tank, {wanted:.2} asked",
        tank_at(&hot, six)
    );

    // 70 °C: above what the tank may hold. Impossible, and the page is told
    // what is reachable.
    let seventy = judged(70.0);
    assert_eq!(seventy.verdict, Verdict::Impossible);
    assert_eq!(seventy.reachable_c, Some(tank.t_max_c));
}
