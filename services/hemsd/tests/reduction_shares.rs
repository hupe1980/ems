//! While § 14a is in force, the page can show who gets what of the ceiling.
//!
//! pv magazine (9 November 2026) found a market leader splitting the limit
//! equally across every EEBUS device, measure-only ones included. hems shares
//! by need; the page draws `reduction.shares`, so the document has to carry
//! them, from the guard's own verdict, on the winter day's evening.

use hemsd::home::GoalSet;
use hemsd::{HouseholdConfig, Scenario};

#[test]
fn a_reduction_the_devices_must_share_says_how_it_is_shared() {
    // The deadline evening: the car arrives as the reduction starts and shares
    // the ceiling with the heat pump.
    let scenario = Scenario::winter_evening_deadline(HouseholdConfig::default());
    let (from, until, _) = scenario.grid_event.expect("a reduction");
    let demo = hemsd::demo::Demo::record(&scenario).expect("the day simulates");
    let mut shared = 0;
    for minute in from.whole_minutes()..until.whole_minutes() {
        let minute = usize::try_from(minute).unwrap();
        let home = serde_json::to_value(demo.home_at(minute, GoalSet::default())).unwrap();
        let reduction = &home["reduction"];
        let Some(budget) = reduction["budget_kw"].as_f64() else {
            continue;
        };
        let shares = reduction["shares"].as_object().expect("shares");
        if !shares.is_empty() {
            shared += 1;
            let total: f64 = shares.values().filter_map(serde_json::Value::as_f64).sum();
            assert!(
                total <= budget + 1e-6,
                "shares {total} over the budget {budget}"
            );
        }
        for (device, kw) in shares {
            let kw = kw.as_f64().unwrap();
            assert!(
                (0.0..=budget + 1e-6).contains(&kw),
                "{device}: {kw} of {budget}"
            );
            assert!(
                home["devices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["id"] == device.as_str()),
                "{device} is a device the page knows"
            );
        }
    }
    assert!(
        shared > 0,
        "the evening the devices compete, the page can say how"
    );
}
