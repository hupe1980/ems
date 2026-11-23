//! Whole-day electricity-cost parity for the supported roof and fixed-service topology.
use super::super::{Clipping, FeedIn, Scored, Unplanned, report_and_bill};
use super::*;
use crate::forecasting::Weather;
use crate::month::IntervalTerms;
use crate::{Household, HouseholdConfig, Scenario};
use ems_core::prelude::Slot;
use ems_forecast::ArrayModel;

fn compare(scenario: &Scenario, shadow: bool) -> Comparison {
    let date = scenario.date;
    let start = ems_core::calendar::day_start_utc(date);
    let store = crate::store::Store::in_memory().unwrap();
    let roof = scenario.config.pv.unwrap();
    let array = ArrayModel::new(roof.kwp, roof.ac_nominal, roof.tilt_deg, roof.azimuth_deg);
    let weather = Weather::new(scenario.weather, scenario.cloudiness, scenario.outdoor_c);
    let quarter = std::time::Duration::from_mins(15);
    let mut meter = PvSim::default();
    let mut twin = Capture::default();
    let energy = |watts: Power| Decimal::try_from(watts.kw() * 0.25).unwrap().round_dp(6);
    for (i, price) in scenario.prices_ct.iter().enumerate() {
        let at = start + Duration::minutes(15 * i64::try_from(i).unwrap());
        let slot = Slot::containing(at);
        let load = weather.load_at(at, crate::scenario::household_load(slot));
        let available = weather.production_at(&array, scenario.config.location, at);
        let baseline = available.min(load + Power::from_kw(3.));
        // A real alternative leaves household demand unchanged and exports less.
        let planned = baseline.min(load.max(baseline * 0.75));
        let actual_ceiling = if shadow { baseline } else { planned };
        let production = meter
            .step(available, actual_ceiling, Duration::minutes(15))
            .0
            .outflow();
        let grid = load - production;
        twin.observe(
            Some(Sample {
                grid,
                storage: None,
                production,
                available,
                planned,
                baseline,
            }),
            shadow,
            quarter,
        );
        let mut registers = ems_grid::mispel::QuarterHour::empty(slot);
        registers.grid_draw = energy(grid.inflow());
        registers.grid_feed_in = energy(grid.outflow());
        registers.anzulegender_wert = Some(8.into());
        let row = Recorded {
            registers,
            production: Some(energy(production)),
            watched: shadow,
            terms: Some(IntervalTerms {
                export_ct: Some(8.into()),
                contract: None,
                import_ct: (*price).into(),
                annual_fixed_eur: 365.into(),
            }),
            storage: None,
            replay: twin.close(),
        };
        store.put_quarter_hour(&row, at).unwrap();
    }
    report_and_bill(
        &store,
        "reference",
        date,
        Unplanned::watching(),
        Clipping::default(),
        FeedIn::default(),
        &Scored::default(),
    )
    .unwrap()
    .unwrap()
    .comparison
    .unwrap()
}

#[test]
fn seven_reference_weather_and_price_traces_match_the_driven_meter_in_both_modes() {
    let full = HouseholdConfig::default();
    let fixed = HouseholdConfig {
        battery: None,
        evse: None,
        heat_pump: None,
        dhw: None,
        dishwasher: None,
        ..full.clone()
    };
    assert!(supported(&Household::build(&fixed).unwrap().site));
    assert!(!supported(&Household::build(&full).unwrap().site));
    for (name, scenario) in [
        ("winter", Scenario::winter_with_grid_event(fixed.clone())),
        ("summer", Scenario::summer_surplus(fixed.clone())),
        ("deadline", Scenario::winter_evening_deadline(fixed.clone())),
        ("shared", Scenario::winter_evening_no_store(&fixed)),
        ("offline", Scenario::summer_without_a_planner(fixed.clone())),
        ("autumn", Scenario::autumn_without_a_planner(fixed.clone())),
        ("capped", Scenario::summer_capped(&fixed)),
    ] {
        let managed = compare(&scenario, false);
        let shadow = compare(&scenario, true);
        assert_eq!(managed, shadow, "{name}");
        assert_eq!(managed.error_ct, 0, "{name}");
    }
}
