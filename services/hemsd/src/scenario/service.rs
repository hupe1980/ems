//! Compare minute-by-minute service before pricing a pair of simulated households.

use super::{CONTROL_PERIOD, Scenario};
use crate::month::{IntervalTerms, MeteredBill};
use crate::runtime::day::replay::{Comparison, ComparisonGap, GapReason, GridReplay};
use crate::store::Recorded;
use ems_core::prelude::{Energy, Power, Slot};
use ems_grid::mispel::QuarterHour;
use ems_tariff::PriceStack;
use rust_decimal::prelude::ToPrimitive;
use time::OffsetDateTime;

/// End-of-tick device state. All energies are Wh. Water deficits are cumulative.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub at: OffsetDateTime,
    pub grid: Power,
    pub outside_band_k: f64,
    pub water_short: Energy,
    pub vehicle: Option<Energy>,
    pub tank: Energy,
    pub battery: Energy,
    pub programme_done: bool,
    pub limited: bool,
}

impl Frame {
    fn complete(self) -> bool {
        self.grid.is_finite()
            && self.outside_band_k.is_finite()
            && self.water_short.is_finite()
            && self.tank.is_finite()
            && self.battery.is_finite()
            && self.vehicle.is_some_and(Energy::is_finite)
    }
}

/// Directional meter charges and the eligible comparison for a simulated day.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Trial {
    /// Gross measured tariff charges on the managed arm.
    pub bill: MeteredBill,
    /// Both bills and observed model error where service qualified.
    pub comparison: Option<Comparison>,
    /// First interval or service observation preventing comparison.
    pub gap: Option<ComparisonGap>,
}

fn service(
    scenario: &Scenario,
    planned: &Frame,
    baseline: &Frame,
    tank_deadline: Option<(OffsetDateTime, Energy)>,
    end: OffsetDateTime,
) -> Option<GapReason> {
    let at = planned.at + CONTROL_PERIOD;
    let states = [planned, baseline];
    let terminal = at == end;
    let departure = scenario.ev.filter(|ev| {
        let deadline = scenario.start() + ev.departure;
        planned.at < deadline && deadline <= at
    });
    if scenario.community.is_some() {
        return Some(GapReason::SharedSettlement);
    }
    if states.iter().any(|s| s.limited) {
        return Some(GapReason::GridLimits);
    }
    if states.iter().any(|s| s.outside_band_k > 0.1) {
        return Some(GapReason::ThermalComfort);
    }
    if states.iter().any(|s| s.water_short.get() > 1.0)
        || tank_deadline.is_some_and(|(by, target)| {
            planned.at < by && by <= at && states.iter().any(|s| target.get() - s.tank.get() > 1.0)
        })
    {
        return Some(GapReason::HotWater);
    }
    if departure.is_some() || terminal {
        let (Some(a), Some(b)) = (planned.vehicle, baseline.vehicle) else {
            return Some(GapReason::UnverifiedService);
        };
        if (a - b).abs().get() > 1.0
            || departure.is_some_and(|e| (e.energy_target - a.min(b)).get() > 1.0)
        {
            return Some(GapReason::VehicleEnergy);
        }
    }
    if terminal {
        if (planned.battery - baseline.battery).abs().get() > 1.0 {
            return Some(GapReason::BatteryInventory);
        }
        if (planned.tank - baseline.tank).abs().get() > 1.0 {
            return Some(GapReason::HotWater);
        }
        if scenario.dishwasher.is_some() && states.iter().any(|s| !s.programme_done) {
            return Some(GapReason::ApplianceProgramme);
        }
    }
    None
}

pub(crate) fn close(
    scenario: &Scenario,
    managed: &[Frame],
    autonomous: &[Frame],
    registers: &[QuarterHour],
    prices: &PriceStack,
    tank_deadline: Option<(OffsetDateTime, Energy)>,
) -> Trial {
    let start = scenario.start();
    let end = ems_core::calendar::day_end_utc(scenario.date);
    let mut gap = None;
    let mut rows = Vec::new();
    let mut quarter = Slot::containing(start);
    let mut offset = 0;
    while quarter.start() < end {
        let mut energy = [[0.0; 2]; 2];
        let mut at = quarter.start();
        while at < quarter.end() {
            let reason = match (managed.get(offset), autonomous.get(offset)) {
                (Some(a), Some(b)) if a.at == at && b.at == at && a.complete() && b.complete() => {
                    for (sums, frame) in energy.iter_mut().zip([a, b]) {
                        sums[0] += frame.grid.inflow().over(CONTROL_PERIOD).get();
                        sums[1] += frame.grid.outflow().over(CONTROL_PERIOD).get();
                    }
                    service(scenario, a, b, tank_deadline, end)
                }
                _ => Some(GapReason::UnverifiedService),
            };
            if let Some(reason) = reason {
                gap.get_or_insert(ComparisonGap {
                    reason,
                    at_unix_s: at.unix_timestamp(),
                });
            }
            offset += 1;
            at += CONTROL_PERIOD;
        }
        if let Some(register) = registers
            .binary_search_by_key(&quarter, |r| r.slot)
            .ok()
            .map(|i| &registers[i])
        {
            let terms = prices
                .at(quarter)
                .filter(|p| p.price_known)
                .map(|p| IntervalTerms {
                    contract: p.contract.clone(),
                    import_ct: p.import_ct,
                    export_ct: p.export_known.then_some(p.export_ct),
                    annual_fixed_eur: p.annual_fixed_eur,
                });
            let units = |pair: [f64; 2]| -> Option<[i64; 2]> {
                Some([
                    (pair[0] * 1000.0).round().to_i64()?,
                    (pair[1] * 1000.0).round().to_i64()?,
                ])
            };
            let replay = units(energy[0])
                .zip(units(energy[1]))
                .map(|(plan, baseline)| GridReplay { plan, baseline });
            rows.push(Recorded {
                registers: *register,
                terms,
                replay,
                watched: false,
                production: None,
                storage: None,
            });
        }
        quarter = quarter.next();
    }
    let bill = MeteredBill::from_registers(scenario.date, &rows);
    let pricing_gap = ComparisonGap::first(scenario.date, &rows, &bill);
    let gap = gap
        .into_iter()
        .chain(pricing_gap)
        .min_by_key(|g| g.at_unix_s);
    let comparison = gap
        .is_none()
        .then(|| Comparison::close(&rows, &bill))
        .flatten();
    Trial {
        bill,
        comparison,
        gap,
    }
}
