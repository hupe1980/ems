//! Replay each minute of a reference day through the household notification detector.

use hemsd::home::{CarGoal, GoalSet};
use hemsd::runtime::push::{Detector, Signals};
use hemsd::{HouseholdConfig, Scenario};

const AT_MOST: usize = 6;

fn notifications(scenario: &Scenario, goals: GoalSet) -> Vec<(usize, String)> {
    let demo = hemsd::demo::Demo::record(scenario).expect("the day simulates");
    let mut detector = Detector::default();
    let mut sent = Vec::new();
    for minute in 0..24 * 60 {
        let home = serde_json::to_value(demo.home_at(minute, goals)).expect("the document");
        let at = time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(minute as i64);
        for event in detector.observe(&Signals::of(&home), at) {
            if event.kind().is_some() {
                sent.push((minute, format!("{event:?}")));
            }
        }
    }
    sent
}

#[test]
fn a_reference_day_interrupts_the_household_at_most_six_times() {
    let car = GoalSet {
        car: Some(CarGoal {
            price_cap_ct: None,
            target_soc: 0.8,
            by_minute: 7 * 60,
        }),
        tank: None,
    };
    for (name, scenario) in [
        (
            "winter",
            Scenario::winter_with_grid_event(HouseholdConfig::default()),
        ),
        (
            "deadline",
            Scenario::winter_evening_deadline(HouseholdConfig::default()),
        ),
        (
            "summer",
            Scenario::summer_surplus(HouseholdConfig::default()),
        ),
    ] {
        let sent = notifications(&scenario, car);
        assert!(
            sent.len() <= AT_MOST,
            "{name}: {} notifications — {sent:?}",
            sent.len()
        );
    }
}

/// And the winter day's reduction is one of them: the bound is not met by a
/// detector that says nothing.
#[test]
fn the_winter_reduction_is_announced_and_lifted() {
    let sent = notifications(
        &Scenario::winter_with_grid_event(HouseholdConfig::default()),
        GoalSet::default(),
    );
    let said: Vec<&str> = sent.iter().map(|(_, e)| e.as_str()).collect();
    assert!(said.contains(&"ReductionBegan"), "{sent:?}");
    assert!(said.contains(&"ReductionEnded"), "{sent:?}");
}
