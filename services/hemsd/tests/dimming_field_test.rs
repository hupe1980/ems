//! The 9 November 2026 field test of a market-leading energy manager, as tests.
//!
//! A pv magazine reader test found a § 14a reduction handled six ways a
//! household notices. Four of the six are decided by code that
//! already exists in this workspace; each is pinned here under the finding's
//! name so a change that reintroduced it fails by that name. The other two —
//! resuming from the stored failsafe after a restart, and not calling a late
//! device unresponsive — are held by `steuerbox_session.rs`
//! (`a_failsafe_the_operator_writes_survives_the_next_power_cut`),
//! `simulated_days.rs` (`failsafe_minutes == 2` on the winter day) and
//! `drivers.rs` (`a_device_that_nods_and_does_nothing_is_named_rather_than_believed`).

use hems_core::prelude::{AssetId, Power};
use hems_realtime::{Claim, allocate};

fn id(s: &str) -> AssetId {
    AssetId::new(s).expect("a valid identifier")
}

fn granted(grants: &[hems_realtime::Grant], asset: &str) -> f64 {
    grants
        .iter()
        .find(|g| g.asset == id(asset))
        .map_or(0.0, |g| g.power.kw())
}

/// Finding 1: the limit was split **equally** across EEBUS devices. Here it is
/// shared by demand: a device that wants less than an equal share is served in
/// full and the rest goes where it is wanted.
#[test]
fn the_minimum_is_shared_by_demand_and_not_equally() {
    let grants = allocate(
        Power::from_kw(4.2),
        &[
            Claim::new(id("waermepumpe"), Power::from_kw(3.3)),
            Claim::new(id("wallbox"), Power::from_kw(0.0)),
        ],
    );
    assert!(
        (granted(&grants, "waermepumpe") - 3.3).abs() < 1e-9,
        "not 2.1"
    );
    assert!(granted(&grants, "wallbox").abs() < 1e-9);
}

/// Finding 2: a measurement-only device took a share. Here only the
/// steuerbare Verbrauchseinrichtungen of `[A1 2.4]` are classified at all, and
/// the guard builds its claims from that classification alone.
#[test]
fn a_measurement_only_device_gets_no_share() {
    let household = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household");
    let steuve = hems_grid::para14a::classify_on(
        &household.site.assets,
        time::macros::date!(2026 - 11 - 17),
    );
    let classified: Vec<&AssetId> = steuve.iter().flat_map(|s| &s.assets).collect();
    for asset in &household.site.assets {
        if matches!(asset, hems_core::asset::Asset::Meter(_)) {
            assert!(
                !classified.contains(&asset.id()),
                "the meter {} must not be a § 14a device",
                asset.id()
            );
        }
    }
}

/// Finding 3: capacity one device did not use was lost. Here it passes to the
/// device that still wants it.
#[test]
fn capacity_one_device_leaves_is_used_by_another() {
    let grants = allocate(
        Power::from_kw(4.2),
        &[
            Claim::new(id("waermepumpe"), Power::from_kw(1.0)),
            Claim::new(id("wallbox"), Power::from_kw(9.0)),
        ],
    );
    assert!((granted(&grants, "waermepumpe") - 1.0).abs() < 1e-9);
    assert!(
        (granted(&grants, "wallbox") - 3.2).abs() < 1e-9,
        "the unused 1.1 kW is not lost"
    );
}

/// Finding 6: a battery that can charge from the grid was ignored. Here a
/// Stromspeicher is a steuerbare Verbrauchseinrichtung of its own, Fallgruppe d.
#[test]
fn a_grid_chargeable_battery_counts_as_a_controllable_device() {
    let household = hemsd::Household::build(&hemsd::HouseholdConfig::default())
        .expect("the reference household");
    let steuve = hems_grid::para14a::classify_on(
        &household.site.assets,
        time::macros::date!(2026 - 11 - 17),
    );
    assert!(
        steuve
            .iter()
            .any(|s| matches!(s.fallgruppe, hems_core::asset::Fallgruppe::Stromspeicher)),
        "the reference battery is § 14a load: {steuve:?}"
    );
}
