//! Grid-charged cycles whose electricity flows and terminal charge were measured.

use ems_core::prelude::{Power, Soc};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};

use crate::store::Recorded;

/// Tick-level source and destination checks have passed for the whole interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    first: Soc,
    last: Soc,
    lowest: Soc,
    highest: Soc,
    starts_idle: bool,
    ends_idle: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Capture {
    trace: Option<Trace>,
    samples: u32,
}

impl Capture {
    pub(crate) fn observe(&mut self, grid: Power, roof: Power, battery: Power, soc: Soc) {
        if battery > Power::ZERO && (roof != Power::ZERO || grid < battery)
            || battery < Power::ZERO && grid < Power::ZERO
        {
            return;
        }
        let idle = battery == Power::ZERO;
        let trace = self.trace.get_or_insert(Trace {
            first: soc,
            last: soc,
            lowest: soc,
            highest: soc,
            starts_idle: idle,
            ends_idle: idle,
        });
        if soc < trace.lowest {
            trace.lowest = soc;
        }
        if soc > trace.highest {
            trace.highest = soc;
        }
        trace.last = soc;
        trace.ends_idle = idle;
        self.samples += 1;
    }

    pub(crate) fn complete(&self, period: std::time::Duration) -> Option<Trace> {
        (period.checked_mul(self.samples) == Some(std::time::Duration::from_mins(15)))
            .then_some(self.trace)
            .flatten()
    }
}

/// AC-side electricity costs for completed measured charge/discharge blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Arbitrage {
    /// Count of closed blocks contributing to this margin.
    pub blocks: u16,
    /// Grid electricity that entered the battery, Wh.
    pub bought_wh: i64,
    /// Electricity returned to household loads, Wh.
    pub served_wh: i64,
    /// Grid charge cost at the actual gross import prices.
    pub bought_ct: i64,
    /// Energy-weighted grid charging rate, thousandths of a cent per kWh.
    pub buy_rate_mct_per_kwh: i64,
    /// Energy-weighted avoided import rate, thousandths of a cent per kWh.
    pub use_rate_mct_per_kwh: i64,
    /// Cost of the grid import the battery displaced when discharging.
    pub avoided_ct: i64,
}

#[derive(Default)]
struct Block {
    bought: Decimal,
    served: Decimal,
    cost: Decimal,
    value: Decimal,
}

impl Arbitrage {
    pub(crate) fn closed(rows: &[Recorded]) -> Option<Self> {
        let mut total = Block::default();
        let mut blocks = 0_u16;
        let mut active = None;
        let mut next = None;
        for r in rows {
            let q = &r.registers;
            if next != Some((q.slot, r.watched)) {
                active = None;
            }
            next = Some((q.slot.offset(1), r.watched));
            let (Some(t), Some(terms), Some(draw), Some(feed)) = (
                r.storage,
                r.terms.as_ref(),
                q.storage_consumption,
                q.storage_generation,
            ) else {
                active = None;
                continue;
            };
            if active.is_none() && t.starts_idle && draw > Decimal::ZERO && feed == Decimal::ZERO {
                active = Some((t.first, Block::default(), false));
            }
            let Some((opening, block, rose)) = &mut active else {
                continue;
            };
            if t.lowest < *opening {
                active = None;
                continue;
            }
            *rose |= t.highest > *opening;
            block.bought = block.bought.checked_add(draw)?;
            block.served = block.served.checked_add(feed)?;
            block.cost = block.cost.checked_add(draw.checked_mul(terms.import_ct)?)?;
            block.value = block
                .value
                .checked_add(feed.checked_mul(terms.import_ct)?)?;
            if t.ends_idle && t.last == *opening && *rose && block.served > Decimal::ZERO {
                // AC discharge in excess of the measured charge is inconsistent
                // with a round trip to the same stored energy.
                if block.bought >= block.served {
                    blocks = blocks.checked_add(1)?;
                    total.bought = total.bought.checked_add(block.bought)?;
                    total.served = total.served.checked_add(block.served)?;
                    total.cost = total.cost.checked_add(block.cost)?;
                    total.value = total.value.checked_add(block.value)?;
                }
                active = None;
            }
        }
        let rate = |cost: Decimal, energy: Decimal| {
            cost.checked_div(energy)?
                .checked_mul(1000.into())?
                .round()
                .to_i64()
        };
        (blocks > 0).then_some(Self {
            blocks,
            bought_wh: (total.bought.checked_mul(1000.into())?).round().to_i64()?,
            served_wh: (total.served.checked_mul(1000.into())?).round().to_i64()?,
            bought_ct: total.cost.round().to_i64()?,
            buy_rate_mct_per_kwh: rate(total.cost, total.bought)?,
            use_rate_mct_per_kwh: rate(total.value, total.served)?,
            avoided_ct: total.value.round().to_i64()?,
        })
    }
}

/// Discharge from other storage, generators or vehicles obscures the source.
pub(crate) fn supported(site: &ems_core::prelude::Site) -> bool {
    use ems_core::prelude::Asset;
    let mut roofs = 0;
    let mut batteries = 0;
    for device in &site.assets {
        match device {
            Asset::Pv(_) => roofs += 1,
            Asset::Battery(_) => batteries += 1,
            Asset::Meter(_) | Asset::HeatPump(_) | Asset::Dhw(_) | Asset::Load(_) => (),
            _ => return false,
        }
    }
    roofs <= 1 && batteries == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ems_core::prelude::Slot;
    use time::macros::datetime;

    fn sample(buy: bool) -> Trace {
        let low = Soc::new(0.3).unwrap();
        let high = Soc::new(0.5).unwrap();
        Trace {
            first: if buy { low } else { high },
            last: if buy { high } else { low },
            lowest: low,
            highest: high,
            starts_idle: buy,
            ends_idle: !buy,
        }
    }

    #[test]
    fn a_completed_cycle_keeps_losses_and_rejects_inventory_or_meter_gaps() {
        let slot = Slot::containing(datetime!(2026-11-02 08:00 UTC));
        let mut rows: Vec<_> = [true, false]
            .into_iter()
            .enumerate()
            .map(|(index, buy)| {
                let mut registers = ems_grid::mispel::QuarterHour::empty(
                    slot.offset(i64::try_from(index).unwrap()),
                );
                registers.storage_consumption = Some(if buy { 1.into() } else { Decimal::ZERO });
                registers.storage_generation = Some(if buy {
                    Decimal::ZERO
                } else {
                    Decimal::new(8, 1)
                });
                Recorded {
                    registers,
                    storage: Some(sample(buy)),
                    replay: None,
                    watched: false,
                    production: None,
                    terms: Some(crate::month::IntervalTerms {
                        export_ct: Some(Decimal::ZERO),
                        contract: None,
                        import_ct: (if buy { 30 } else { 35 }).into(),
                        annual_fixed_eur: Decimal::ZERO,
                    }),
                }
            })
            .collect();
        let a = Arbitrage::closed(&rows).unwrap();
        assert_eq!(
            (
                a.blocks,
                a.bought_wh,
                a.served_wh,
                a.bought_ct,
                a.avoided_ct
            ),
            (1, 1000, 800, 30, 28)
        );
        assert_eq!(
            (a.buy_rate_mct_per_kwh, a.use_rate_mct_per_kwh),
            (30_000, 35_000)
        );
        assert!(Arbitrage::closed(&rows[..1]).is_none());
        rows[1].storage.as_mut().unwrap().ends_idle = false;
        assert!(Arbitrage::closed(&rows).is_none());
        rows[1].storage = Some(sample(false));
        rows[1].registers.slot = slot.offset(2);
        assert!(Arbitrage::closed(&rows).is_none());
        rows[1].registers.slot = slot.offset(1);
        rows[0].storage = None;
        assert!(Arbitrage::closed(&rows).is_none());
    }

    #[test]
    fn simultaneous_solar_charge_and_exported_discharge_are_unattributable() {
        let mut c = Capture::default();
        let hour = std::time::Duration::from_mins(15);
        let w = Power::new;
        c.observe(w(2000.), w(-100.), w(1000.), Soc::FULL);
        c.observe(w(-200.), w(0.), w(-1000.), Soc::FULL);
        c.observe(w(200.), w(0.), w(1000.), Soc::FULL);
        assert!(c.complete(hour).is_none());
        c.observe(w(2000.), w(0.), w(1000.), Soc::FULL);
        assert!(c.complete(hour).is_some());
    }
}
