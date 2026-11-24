//! Matched grid-energy replays over verified PV, storage and household service.

use ems_core::prelude::{Asset, Energy, Power, Site, Soc};
use ems_sim::{BatterySim, PvSim};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};
use time::Duration;

use crate::drivers::Observed;
use crate::month::MeteredBill;
use crate::store::Recorded;

/// Assets for which the gateway can test service equality from local telemetry.
pub(crate) fn supported(site: &Site) -> bool {
    let mut roofs = 0;
    let mut packs = 0;
    let mut cars = 0;
    for asset in &site.assets {
        match asset {
            Asset::Pv(_) => roofs += 1,
            Asset::Battery(_) => packs += 1,
            Asset::Evse(_) => cars += 1,
            Asset::Meter(_) => (),
            Asset::Load(load) if matches!(load.kind, ems_core::asset::LoadKind::Fixed) => (),
            _ => return false,
        }
    }
    roofs <= 1 && packs <= 1 && cars <= 1
}

#[derive(Clone, Copy)]
struct StoredEnergy {
    capacity: Energy,
    charge: Power,
    discharge: Power,
    floor: Soc,
    ceiling: Soc,
    soc: Soc,
    actual: Power,
}

impl StoredEnergy {
    fn model(self) -> BatterySim {
        BatterySim {
            stored: self.capacity * self.soc.fraction(),
            max_discharge: self.discharge,
            soc_min: self.floor,
            soc_max: self.ceiling,
            ..BatterySim::new(self.capacity, self.charge)
        }
    }

    /// Both alternatives must reproduce the measured AC flow from the
    /// observed state. Then neither uses more inventory, cycles or reserve.
    fn same_service(self, demand: [Power; 2], dt: Duration) -> bool {
        let model = self.model();
        demand.into_iter().all(|load| {
            let predicted = model.clone().step(-load, dt);
            (predicted - self.actual).abs().get() <= 1e-6
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    grid: Power,
    storage: Option<StoredEnergy>,
    production: Power,
    available: Power,
    planned: Power,
    baseline: Power,
}

fn unplugged(
    presence: &ems_drv::VehiclePresence,
    port: &ems_core::measurement::Measurement,
    now: time::OffsetDateTime,
) -> bool {
    !presence.connected
        && presence.at <= now
        && now - presence.at <= Duration::MINUTE
        && port.at <= now
        && now - port.at <= Duration::MINUTE
        && port.quality == metering::QualityFlag::Measured
        && port.power == Some(Power::ZERO)
}

impl Sample {
    pub(crate) fn from_tick(
        observed: &Observed,
        managed: &crate::runtime::control::Managed,
        now: time::OffsetDateTime,
        decision: &ems_realtime::arbiter::Decision,
    ) -> Option<Self> {
        let meter = observed.state.grid?;
        if meter.quality != metering::QualityFlag::Measured {
            return None;
        }
        let grid = meter.power?;
        if let Some(id) = &managed.evse {
            let presence = observed.vehicles.get(id)?;
            let port = observed.state.asset(id)?;
            if !unplugged(presence, port, now) {
                return None;
            }
        }
        let storage = if let Some(id) = &managed.battery {
            let Asset::Battery(pack) = managed.site.asset(id)? else {
                return None;
            };
            let battery = observed.state.asset(id)?;
            let actual = battery.power?;
            if battery.quality != metering::QualityFlag::Measured
                || decision.commanded.get(id).copied()? != actual
            {
                return None;
            }
            Some(StoredEnergy {
                capacity: pack.capacity,
                charge: pack.max_charge,
                discharge: pack.max_discharge,
                floor: if pack.reserve_soc > pack.soc_min {
                    pack.reserve_soc
                } else {
                    pack.soc_min
                },
                ceiling: pack.soc_max,
                soc: battery.soc?,
                actual,
            })
        } else {
            None
        };
        let consumption = grid - storage.map_or(Power::ZERO, |b| b.actual);
        let Some(pv) = managed.pv.as_ref() else {
            return (consumption >= Power::ZERO).then_some(Self {
                grid,
                storage,
                production: Power::ZERO,
                available: Power::ZERO,
                planned: Power::ZERO,
                baseline: Power::ZERO,
            });
        };
        if !observed.curtailable.contains(pv) || observed.assumed_available.contains(pv) {
            return None;
        }
        let roof = observed.state.asset(pv)?;
        if roof.quality != metering::QualityFlag::Measured
            || roof.estimated_availability
            || roof.power? > Power::ZERO
        {
            return None;
        }
        let production = roof.power?.outflow();
        let available = roof.available_power?;
        if available < Power::ZERO || consumption + production < Power::ZERO {
            return None;
        }
        let planned = decision.commanded.get(pv)?.outflow();
        let baseline = decision
            .verdict
            .envelopes
            .get(pv)?
            .floor
            .outflow()
            .min(available);
        Some(Self {
            grid,
            storage,
            production,
            available,
            planned,
            baseline,
        })
    }
}

/// Quarter-hour integrals from the same measured load and available sunshine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridReplay {
    /// Import and export for the plan, in micro-kWh.
    pub plan: [i64; 2],
    /// Import and export under autonomous self-consumption, in micro-kWh.
    pub baseline: [i64; 2],
}

#[derive(Debug, Default)]
pub(crate) struct Capture {
    twins: Option<[PvSim; 2]>,
    elapsed: std::time::Duration,
    energy_wh: [[f64; 2]; 2],
}

impl Capture {
    pub(crate) fn observe(
        &mut self,
        sample: Option<Sample>,
        shadow: bool,
        period: std::time::Duration,
    ) {
        let Some(s) = sample else {
            self.twins = None;
            return;
        };
        let twins = self.twins.get_or_insert(
            [PvSim {
                producing: s.production,
                ..PvSim::default()
            }; 2],
        );
        let gross_load = s.grid + s.production;
        let Ok(dt) = Duration::try_from(period) else {
            return;
        };
        let grid =
            |sim: &mut PvSim, ceiling: Power| gross_load + sim.step(s.available, ceiling, dt).0;
        let planned = grid(&mut twins[0], s.planned);
        let autonomous = grid(&mut twins[1], s.baseline);
        if let Some(battery) = s.storage
            && !battery.same_service([planned - battery.actual, autonomous - battery.actual], dt)
        {
            return;
        }
        if twins.iter().any(|sim| sim.producing > s.baseline) {
            return;
        }
        // Independent control may enforce an unpublished grid envelope. Only
        // compare a shadow plan whose exchange stays inside the observed bounds.
        if shadow && (planned.inflow() > s.grid.inflow() || planned.outflow() > s.grid.outflow()) {
            return;
        }
        for (sum, power) in self.energy_wh.iter_mut().zip([planned, autonomous]) {
            sum[0] += power.inflow().get() * period.as_secs_f64() / 3600.0;
            sum[1] += power.outflow().get() * period.as_secs_f64() / 3600.0;
        }
        self.elapsed += period;
    }

    pub(crate) fn close(&mut self) -> Option<GridReplay> {
        let elapsed = std::mem::take(&mut self.elapsed);
        let energy = std::mem::take(&mut self.energy_wh);
        if elapsed != std::time::Duration::from_mins(15) {
            return None;
        }
        let units = |[import, export]: [f64; 2]| -> Option<[i64; 2]> {
            Some([
                (import * 1000.0).round().to_i64()?,
                (export * 1000.0).round().to_i64()?,
            ])
        };
        Some(GridReplay {
            plan: units(energy[0])?,
            baseline: units(energy[1])?,
        })
    }
}

/// Evidence missing from the first affected quarter of a closed day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    /// The gateway has no register for the quarter.
    MissingInterval,
    /// The interval cannot be priced under both alternatives.
    MissingPrice,
    /// The available readings do not establish equal service.
    UnverifiedService,
    /// Responsibility for control changed during the day.
    ControlChanged,
    /// The simulation arms operated under different grid control regimes.
    GridLimits,
    /// Room temperature left the household comfort band.
    ThermalComfort,
    /// Hot-water delivery, deadline or terminal tank inventory differed.
    HotWater,
    /// Vehicle energy at departure or at day close did not qualify.
    VehicleEnergy,
    /// End-of-day battery inventories differed.
    BatteryInventory,
    /// A requested appliance programme did not finish.
    ApplianceProgramme,
    /// The supply meter alone cannot price the community allocation.
    SharedSettlement,
}

/// Recorded explanation for an unavailable comparison on a day receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ComparisonGap {
    /// Start of the first affected observation interval in Unix seconds.
    pub at_unix_s: i64,
    /// Evidence that prevented comparison.
    pub reason: GapReason,
}

impl ComparisonGap {
    pub(crate) fn first(date: time::Date, rows: &[Recorded], bill: &MeteredBill) -> Option<Self> {
        use ems_core::prelude::Slot;
        let first = Slot::containing(ems_core::calendar::day_start_utc(date));
        let watched = rows.first().is_some_and(|r| r.watched);
        let mut rows = rows.iter();
        for i in 0..bill.expected_intervals {
            let slot = first.offset(i64::from(i));
            let row = rows.next().filter(|row| row.registers.slot == slot);
            let reason = match row {
                None => GapReason::MissingInterval,
                Some(row) if row.watched != watched => GapReason::ControlChanged,
                Some(row) => match (&row.terms, row.replay) {
                    (None, _) => GapReason::MissingPrice,
                    (Some(_), None) => GapReason::UnverifiedService,
                    (Some(terms), Some(replay)) => {
                        let has_export = !row.registers.grid_feed_in.is_zero()
                            || replay.plan[1] != 0
                            || replay.baseline[1] != 0;
                        if terms.export_ct.is_some() || !has_export {
                            continue;
                        }
                        GapReason::MissingPrice
                    }
                },
            };
            return Some(Self {
                at_unix_s: slot.start().unix_timestamp(),
                reason,
            });
        }
        None
    }
}

/// Day-level grid-energy comparison, including fixed charges in both arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Comparison {
    /// What the guarded plan cost, or would have cost on a shadow gateway.
    pub with_plan_ct: i64,
    /// What the same service cost on autonomous self-consumption.
    pub without_plan_ct: i64,
    /// Sum of absolute interval cost errors in the arm the meter observed, rounded up.
    pub error_ct: i64,
}

impl Comparison {
    pub(crate) fn delta(&self) -> i128 {
        i128::from(self.without_plan_ct) - i128::from(self.with_plan_ct)
    }

    pub(crate) fn conclusive(&self) -> bool {
        self.delta().abs() > i128::from(self.error_ct)
    }

    pub(crate) fn close(rows: &[Recorded], bill: &MeteredBill) -> Option<Self> {
        let fixed = bill.fixed_ct?;
        let measured = bill.energy_ct?.checked_add(fixed)?;
        let watched = rows.first()?.watched;
        if rows.len() != usize::from(bill.expected_intervals) || rows.len() > 100 {
            return None;
        }
        let mut plan = Decimal::ZERO;
        let mut baseline = Decimal::ZERO;
        let mut error = Decimal::ZERO;
        for row in rows {
            if row.watched != watched {
                return None;
            }
            let terms = row.terms.as_ref()?;
            let q = &row.registers;
            let net_ct = |draw, feed| terms.net_charge(draw, feed);
            let price =
                |[draw, feed]: [i64; 2]| net_ct(Decimal::new(draw, 6), Decimal::new(feed, 6));
            let replay = row.replay?;
            let p = price(replay.plan)?;
            let b = price(replay.baseline)?;
            plan = plan.checked_add(p)?;
            baseline = baseline.checked_add(b)?;
            let actual = net_ct(q.grid_draw, q.grid_feed_in)?;
            let modelled = if watched { b } else { p };
            error = error.checked_add(actual.checked_sub(modelled)?.abs())?;
        }
        let projected = |cost: Decimal| cost.round().to_i64()?.checked_add(fixed);
        Some(Self {
            with_plan_ct: if watched { projected(plan)? } else { measured },
            without_plan_ct: if watched {
                measured
            } else {
                projected(baseline)?
            },
            error_ct: error.ceil().to_i64()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ems_core::prelude::Slot;
    use time::macros::date;

    #[test]
    fn an_unplugged_port_requires_current_presence_and_zero_measured_power() {
        let now = time::macros::datetime!(2026-11-23 12:00 UTC);
        let mut port = ems_core::measurement::Measurement::at(now);
        let mut presence = ems_drv::VehiclePresence {
            connected: false,
            at: now,
            capacity_wh: None,
            soc: None,
        };
        assert!(!unplugged(&presence, &port, now));
        port.power = Some(Power::ZERO);
        assert!(unplugged(&presence, &port, now));
        assert!(!unplugged(&presence, &port, now + Duration::minutes(2)));
        for offset in [-61, 1] {
            port.at = now + Duration::seconds(offset);
            assert!(!unplugged(&presence, &port, now));
        }
        port.at = now;
        presence.connected = true;
        assert!(!unplugged(&presence, &port, now));
    }

    #[test]
    fn only_battery_trajectories_with_equal_measured_service_can_be_priced() {
        let w = Power::new;
        let mut pack = StoredEnergy {
            capacity: Energy::from_kwh(5.),
            charge: w(2000.),
            discharge: w(1000.),
            floor: Soc::new(0.2).unwrap(),
            ceiling: Soc::FULL,
            soc: Soc::new(0.5).unwrap(),
            actual: w(-1000.),
        };
        // The hardware saturates at 1 kW in both runs: identical AC throughput
        // and identical energy delivered despite different PV production.
        assert!(pack.same_service([w(1500.), w(1200.)], Duration::SECOND));
        assert!(!pack.same_service([w(1500.), w(700.)], Duration::SECOND));
        pack.soc = pack.floor;
        assert!(!pack.same_service([w(1500.), w(1200.)], Duration::SECOND));
        pack.actual = Power::ZERO;
        assert!(pack.same_service([w(1500.), w(1200.)], Duration::SECOND));
        assert!(!pack.same_service([w(-500.), w(1200.)], Duration::SECOND));
    }

    #[test]
    fn every_second_of_same_service_and_observed_shadow_limits_is_required() {
        let mut capture = Capture::default();
        let period = std::time::Duration::from_secs(5);
        let sample = Sample {
            grid: Power::new(1000.),
            storage: None,
            production: Power::new(1000.),
            available: Power::new(1000.),
            planned: Power::new(1000.),
            baseline: Power::new(1000.),
        };
        for _ in 0..180 {
            capture.observe(Some(sample), true, period);
        }
        let quarter = capture.close().unwrap();
        assert_eq!(quarter.plan, [250_000, 0]);
        assert_eq!(quarter.plan, quarter.baseline);
        for _ in 0..179 {
            capture.observe(Some(sample), true, period);
        }
        capture.observe(None, true, period);
        assert!(capture.close().is_none());
        let overdraw = Sample {
            planned: Power::ZERO,
            ..sample
        };
        for _ in 0..180 {
            capture.observe(Some(overdraw), true, period);
        }
        assert!(capture.close().is_none());
    }

    #[test]
    fn observed_arm_error_does_not_cancel_or_hide_losses_or_missing_evidence() {
        let date = date!(2026 - 11 - 12);
        let start = Slot::containing(ems_core::calendar::day_start_utc(date));
        let mut rows: Vec<_> = (0..96)
            .map(|i| {
                let mut registers = ems_grid::mispel::QuarterHour::empty(start.offset(i));
                registers.grid_draw = Decimal::ONE;
                Recorded {
                    registers,
                    storage: None,
                    production: None,
                    watched: false,
                    replay: Some(GridReplay {
                        plan: [1_000_000, 0],
                        baseline: [900_000, 0],
                    }),
                    terms: Some(crate::month::IntervalTerms {
                        export_ct: Some(Decimal::ZERO),
                        contract: None,
                        import_ct: 25.into(),
                        annual_fixed_eur: 365.into(),
                    }),
                }
            })
            .collect();
        let bill = MeteredBill::from_registers(date, &rows);
        let c = Comparison::close(&rows, &bill).unwrap();
        assert_eq!(
            (c.with_plan_ct, c.without_plan_ct, c.error_ct),
            (2500, 2260, 0)
        );
        for r in &mut rows {
            r.watched = true;
        }
        let shadow = Comparison::close(&rows, &bill).unwrap();
        assert_eq!(
            (shadow.with_plan_ct, shadow.without_plan_ct, shadow.error_ct),
            (2500, 2500, 240)
        );
        rows[0].watched = false;
        assert_eq!(
            ComparisonGap::first(date, &rows, &bill).unwrap().reason,
            GapReason::ControlChanged
        );
        assert!(Comparison::close(&rows, &bill).is_none());
        rows[0].watched = true;
        for (i, r) in rows.iter_mut().enumerate() {
            r.replay.as_mut().unwrap().baseline[0] = if i % 2 == 0 { 1_100_000 } else { 900_000 };
        }
        assert_eq!(Comparison::close(&rows, &bill).unwrap().error_ct, 240);
        rows[1].replay = None;
        assert!(Comparison::close(&rows, &bill).is_none());
        assert!(Comparison::close(&rows[1..], &bill).is_none());
        let gap = ComparisonGap::first(date, &rows, &bill).unwrap();
        assert_eq!(gap.reason, GapReason::UnverifiedService);
        assert_eq!(gap.at_unix_s, start.next().start().unix_timestamp());
        assert_eq!(
            ComparisonGap::first(date, &rows[1..], &bill)
                .unwrap()
                .reason,
            GapReason::MissingInterval
        );
        rows[0].terms.as_mut().unwrap().export_ct = None;
        rows[0].replay.as_mut().unwrap().plan[1] = 1;
        assert_eq!(
            ComparisonGap::first(date, &rows, &bill).unwrap().reason,
            GapReason::MissingPrice
        );
    }
}

#[cfg(test)]
mod reference;
