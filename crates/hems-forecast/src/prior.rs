//! What a box assumes about a household before it has measured one.
//!
//! A box on its first evening has one meter reading, and persistence from it is
//! the wrong shape for the one quantity that matters: a household's demand has a
//! strong daily shape, and the reading a box starts from is usually the night
//! trough. Planned against a flat line at the trough, the first evening defers
//! flexible load into hours it believes are quiet and cheap, and a box that
//! knows nothing comes out worse than no box at all.
//!
//! The shape to hang that reading on is published: BDEW's **H25** household
//! profile, the 2025 revision of the standard load profiles that every German
//! network operator may balance households on (BDEW, *Hinweise zu den
//! aktualisierten Standardlastprofilen Strom*, 17.03.2025). Twelve months, three
//! day types — Werktag, Samstag, Sonn- und Feiertag on the Bundesland's own
//! calendar — and 96 quarter hours, normalised to one million kWh a year and
//! dynamised over the year by BDEW's quartic. `metering` holds that model
//! ([`DynamicSlpProfile`]); this module supplies the table and scales it to one
//! household.
//!
//! # Why H25 and not P25, on a house with a roof
//!
//! BDEW balances a household **with** photovoltaics on P25, whose values are the
//! metered net draw with the roof already taken off. This forecast is the
//! opposite quantity — what the house uses that nobody controls, before the roof
//! — and the planner subtracts its own PV forecast. P25 would count the roof
//! twice. H25 describes the gross household load, which is what is wanted here.
//!
//! # How it is scaled
//!
//! Two ways, in order of preference:
//!
//! * a **declared** household consumption, kWh a year — the household's own
//!   *Haushaltsstrom*, without heat pump, car or battery. Not the figure on a
//!   bill that meters a heat pump or a wallbox on the same register: H25
//!   describes household appliances, and scaling it by a bill that includes a
//!   heat pump would overstate the base load several times over;
//! * once the box has metered **one whole day** of any day type, **fitted**:
//!   the household's own learned energy for that day against H25's for a day of
//!   the same type this month. A day is the unit because the shape is what the
//!   prior supplies and the level is what the household does; a whole day
//!   averages away the kettle and the shower that one reading cannot;
//! * before that, **anchored** on the box's own first reading: the scale at which
//!   H25's value for that quarter hour equals what the meter says — but never
//!   below a typical family house ([`TYPICAL_HOUSE_KWH_PER_YEAR`]), and never
//!   above a plausible ceiling.
//!
//! # Why the anchor has a floor
//!
//! One reading is a weak signal, and the two ways it can be wrong do not cost
//! the same. A box that plans for **more** household load than there is spends
//! a little on energy it did not need; a box that plans for **less** defers its
//! flexible load into hours it believes are quiet and cheap, which is exactly
//! how a cold box came to be worse than none. And the reading a box starts from
//! is usually the night trough, which is the direction that under-scales. So the
//! anchor errs high, by the same rule the guard follows for a silent device: the
//! conservative number, said out loud. The asymmetry is measured on the
//! reference household rather than assumed, and is held by a test.
//!
//! The band is wide on purpose ([`PRIOR_SPREAD`]): a standard profile is the
//! average of very many households, and one household's quarter hour scatters
//! far more than an average does. A planner handed a narrow band from a
//! population average would bet a battery on somebody else's evening.

use hems_core::calendar::Bundesland;
use hems_core::prelude::{Power, Slot};
use metering::slp::strom::{DynamicSlpProfile, Dynamization, LoadProfile as Slp, SlpDayType};
use metering::time::holiday::SlpCalendar;
use rust_decimal::prelude::ToPrimitive;

use crate::load::LoadProfile;
use crate::quantile::Band;
use hems_core::prelude::DayType;

/// The H25 table, as BDEW publishes it. Provenance is in the file's header.
const H25_CSV: &str = include_str!("../data/h25.csv");

/// The annual consumption BDEW normalises every profile to, kWh.
pub const NORMALISED_KWH_PER_YEAR: f64 = 1_000_000.0;

/// The relative half-width of a band answered from the prior alone.
///
/// The same width as a quarter hour the household has never been metered
/// through ([`crate::load`]'s unseen widening of its 40 % default): a prior is
/// exactly that, a quarter hour this household has not shown the box yet.
pub const PRIOR_SPREAD: f64 = 0.8;

/// A typical family house's household consumption, kWh a year — the floor an
/// anchored scale is held to.
///
/// BDEW, *Stromverbrauch im Haushalt 2025*, short version of 20.05.2026, slide 10:
/// 4 175 kWh for a household of four or more persons. A box that manages a heat
/// pump, a roof and a wallbox is installed in a house rather than a flat, and
/// BDEW's four-or-more-person figure is the one for it.
pub const TYPICAL_HOUSE_KWH_PER_YEAR: f64 = 4_175.0;

/// The range an anchored or fitted scale is held to, kWh of household
/// consumption a year: from a small flat to a large household with every
/// appliance electric. The anchor additionally never goes below
/// [`TYPICAL_HOUSE_KWH_PER_YEAR`].
pub const ANCHOR_RANGE_KWH: (f64, f64) = (800.0, 9_000.0);

/// BDEW's H25 household profile, loaded and dynamised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardProfile {
    table: DynamicSlpProfile,
    calendar: SlpCalendar,
}

impl StandardProfile {
    /// H25 on `land`'s holiday calendar.
    ///
    /// # Panics
    ///
    /// Never at run time: the table is compiled in and its shape is a test.
    #[must_use]
    pub fn h25(land: Bundesland) -> Self {
        let mut table = DynamicSlpProfile::new(Slp::H25).dynamization(Dynamization::BDEW);
        for line in H25_CSV
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let mut fields = line.split(',');
            let month: u8 = fields
                .next()
                .and_then(|m| m.parse().ok())
                .expect("H25 row: a month");
            let day_type = match fields.next() {
                Some("WT") => SlpDayType::Werktag,
                Some("SA") => SlpDayType::Samstag,
                Some("FT") => SlpDayType::SonnFeiertag,
                other => panic!("H25 row: unknown day type {other:?}"),
            };
            let values = fields
                .map(|v| v.parse().expect("H25 row: a decimal value"))
                .collect();
            table
                .insert(month, day_type, values)
                .expect("H25 row: a 96-value day table");
        }
        debug_assert!(table.is_complete(), "H25 holds all 12 × 3 day tables");
        Self {
            table,
            calendar: SlpCalendar::new(land),
        }
    }

    /// The profile's power in `slot` for a household of `kwh_per_year`.
    ///
    /// `None` only outside the calendar's supported years.
    #[must_use]
    pub fn power_at(&self, slot: Slot, kwh_per_year: f64) -> Option<Power> {
        let quarter_kwh = self
            .table
            .value_at(slot.start(), &self.calendar)?
            .to_f64()?;
        // kWh in a quarter hour, ×4 is the mean kW over it.
        Some(Power::from_kw(
            quarter_kwh * 4.0 * kwh_per_year / NORMALISED_KWH_PER_YEAR,
        ))
    }

    /// Scale to a declared household consumption.
    #[must_use]
    pub fn declared(self, kwh_per_year: f64) -> Prior {
        Prior {
            profile: self,
            kwh_per_year,
            basis: PriorBasis::Declared,
        }
    }

    /// Scale to the household's own whole days, where it has any.
    ///
    /// For each day type `learned` has seen through all of its quarter hours,
    /// the ratio of the household's learned energy for that day to H25's for
    /// the nearest day of the same type up to `at`; the mean of those ratios,
    /// held to [`ANCHOR_RANGE_KWH`]. `None` until one whole day has been seen.
    #[must_use]
    pub fn fitted(self, learned: &LoadProfile, at: Slot, land: Bundesland) -> Option<Prior> {
        let mut implied = Vec::new();
        for day_type in [DayType::Workday, DayType::Saturday, DayType::Sunday] {
            let Some(own_kwh) = learned.day_kwh(day_type) else {
                continue;
            };
            let Some(published) = self.day_kwh_like(day_type, at, land) else {
                continue;
            };
            if published > 0.0 {
                implied.push(own_kwh / published * NORMALISED_KWH_PER_YEAR);
            }
        }
        if implied.is_empty() {
            return None;
        }
        let mean = implied.iter().sum::<f64>() / implied.len() as f64;
        Some(Prior {
            profile: self,
            kwh_per_year: mean.clamp(ANCHOR_RANGE_KWH.0, ANCHOR_RANGE_KWH.1),
            basis: PriorBasis::Fitted,
        })
    }

    /// H25's energy, normalised, on the most recent day of `day_type` up to
    /// and including the day `at` falls in — every quarter hour that day has,
    /// so 23 and 25 hours on the days the clocks change.
    fn day_kwh_like(&self, day_type: DayType, at: Slot, land: Bundesland) -> Option<f64> {
        let mut date = at.local_date();
        for _ in 0..14 {
            let mut s = Slot::containing(hems_core::calendar::day_start_utc(date));
            if DayType::of(s, land) == day_type {
                let mut kwh = 0.0;
                while s.local_date() == date {
                    kwh += self.power_at(s, NORMALISED_KWH_PER_YEAR)?.get() / 1_000.0 * 0.25;
                    s = s.next();
                }
                return Some(kwh);
            }
            date = date.previous_day()?;
        }
        None
    }

    /// Scale so that the profile agrees with one reading taken in `slot` —
    /// never below [`TYPICAL_HOUSE_KWH_PER_YEAR`] and never above the top of
    /// [`ANCHOR_RANGE_KWH`].
    #[must_use]
    pub fn anchored(self, slot: Slot, reading: Power) -> Prior {
        let unit = self
            .power_at(slot, NORMALISED_KWH_PER_YEAR)
            .map_or(0.0, Power::get);
        let implied = if unit > 0.0 {
            reading.get() / unit * NORMALISED_KWH_PER_YEAR
        } else {
            TYPICAL_HOUSE_KWH_PER_YEAR
        };
        Prior {
            profile: self,
            kwh_per_year: implied.clamp(TYPICAL_HOUSE_KWH_PER_YEAR, ANCHOR_RANGE_KWH.1),
            basis: PriorBasis::Anchored,
        }
    }
}

/// Where a prior's scale came from — said on the day report, because a box that
/// planned its first evening on a guess should say whose guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PriorBasis {
    /// The household declared its annual consumption.
    Declared,
    /// Scaled to the household's own metered days.
    Fitted,
    /// Scaled to the box's own first reading.
    Anchored,
}

/// H25, scaled to one household.
#[derive(Debug, Clone, PartialEq)]
pub struct Prior {
    profile: StandardProfile,
    kwh_per_year: f64,
    basis: PriorBasis,
}

impl Prior {
    /// The band the prior gives `slot`: H25's value as the median, widened by
    /// [`PRIOR_SPREAD`].
    #[must_use]
    pub fn band_at(&self, slot: Slot) -> Band {
        let median = self
            .profile
            .power_at(slot, self.kwh_per_year)
            .map_or(0.0, Power::get);
        Band::relative(median, PRIOR_SPREAD)
    }

    /// The household consumption the prior is scaled to, kWh a year.
    #[must_use]
    pub fn kwh_per_year(&self) -> f64 {
        self.kwh_per_year
    }

    /// Where that scale came from.
    #[must_use]
    pub fn basis(&self) -> PriorBasis {
        self.basis
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn slot(at: time::OffsetDateTime) -> Slot {
        Slot::containing(at)
    }

    #[test]
    fn the_table_is_whole_and_sums_to_the_household_it_is_scaled_to() {
        // Every day of 2026, every quarter hour: a profile normalised to one
        // million kWh scaled to 4 000 must deliver about 4 000 over the year.
        // "About" because BDEW normalises before the Dynamisierung and the
        // holidays move days between tables; a transcription error — a
        // missing column, a month shifted by one — moves it by far more.
        let h25 = StandardProfile::h25(Bundesland::Be);
        let mut s = slot(datetime!(2025-12-31 23:00 UTC));
        let end = datetime!(2026-12-31 23:00 UTC);
        let mut kwh = 0.0;
        while s.start() < end {
            kwh += h25.power_at(s, 4_000.0).expect("2026 is supported").get() / 1_000.0 * 0.25;
            s = s.next();
        }
        assert!((3_950.0..4_050.0).contains(&kwh), "{kwh:.0} kWh");
    }

    #[test]
    fn a_winter_workday_peaks_in_the_evening_and_bottoms_out_at_night() {
        // The shape the prior exists for: the night trough a cold box starts
        // from is a fraction of the evening it has to plan for.
        let h25 = StandardProfile::h25(Bundesland::Be);
        let night = h25
            .power_at(slot(datetime!(2026-01-15 02:00 UTC)), 4_000.0)
            .unwrap();
        let evening = h25
            .power_at(slot(datetime!(2026-01-15 17:45 UTC)), 4_000.0)
            .unwrap();
        assert!(
            evening.get() > 2.5 * night.get(),
            "{evening:?} against {night:?}"
        );
    }

    #[test]
    fn a_holiday_reads_the_sunday_table_in_the_land_that_keeps_it() {
        // Fronleichnam 2026 is a Thursday: a holiday in Bayern, a workday in
        // Berlin. The same instant reads two different tables.
        let at = slot(datetime!(2026-06-04 10:00 UTC));
        let by = StandardProfile::h25(Bundesland::By)
            .power_at(at, 4_000.0)
            .unwrap();
        let be = StandardProfile::h25(Bundesland::Be)
            .power_at(at, 4_000.0)
            .unwrap();
        assert_ne!(by, be);
    }

    #[test]
    fn an_anchor_believes_a_large_household_and_never_scales_below_a_typical_house() {
        let at = slot(datetime!(2026-01-15 02:00 UTC));
        let h25 = StandardProfile::h25(Bundesland::Be);
        let large = h25.power_at(at, 6_000.0).unwrap();
        let prior = h25.clone().anchored(at, large);
        assert!((prior.kwh_per_year() - 6_000.0).abs() < 1.0);
        assert!((prior.band_at(at).p50 - large.get()).abs() < 1e-9);
        assert_eq!(prior.basis(), PriorBasis::Anchored);

        // The night trough a box usually starts from implies a small household,
        // and under-scaling is the expensive direction: held at the floor.
        let trough = h25.power_at(at, 2_000.0).unwrap();
        let held = h25.clone().anchored(at, trough);
        assert_eq!(held.kwh_per_year(), TYPICAL_HOUSE_KWH_PER_YEAR);
        let dark = h25.clone().anchored(at, Power::ZERO);
        assert_eq!(dark.kwh_per_year(), TYPICAL_HOUSE_KWH_PER_YEAR);

        // A kettle at 03:00 is not a household of thirty thousand kWh.
        let kettle = h25.anchored(at, Power::from_kw(3.0));
        assert_eq!(kettle.kwh_per_year(), ANCHOR_RANGE_KWH.1);
    }

    #[test]
    fn january_is_not_july() {
        // The defect a transcription is most likely to make — a month's three
        // columns in another month's place — moves the seasons, and a household
        // uses more in January than in July. SR-031.
        let h25 = StandardProfile::h25(Bundesland::Be);
        let day = |at: time::OffsetDateTime| -> f64 {
            let mut s = slot(at);
            let date = s.local_date();
            let mut kwh = 0.0;
            while s.local_date() == date {
                kwh += h25.power_at(s, 4_000.0).unwrap().get() / 1_000.0 * 0.25;
                s = s.next();
            }
            kwh
        };
        let january = day(datetime!(2026-01-14 23:00 UTC));
        let july = day(datetime!(2026-07-14 22:00 UTC));
        assert!(
            january > 1.2 * july,
            "{january:.2} kWh against {july:.2} kWh"
        );
    }

    #[test]
    fn one_whole_metered_day_replaces_the_anchor_with_the_household_itself() {
        // SR-030: as soon as the household has shown the box one whole day, the
        // scale is its own rather than one reading's.
        let mut learned = LoadProfile::new(Bundesland::Be);
        let mut s = slot(datetime!(2026-01-13 23:00 UTC)); // Wednesday, 00:00 Berlin
        let date = s.local_date();
        while s.local_date() == date {
            learned.observe(s, Power::from_kw(0.25)); // 6 kWh that day
            s = s.next();
        }
        let at = slot(datetime!(2026-01-15 12:00 UTC));
        let prior = StandardProfile::h25(Bundesland::Be)
            .fitted(&learned, at, Bundesland::Be)
            .expect("a whole workday was seen");
        assert_eq!(prior.basis(), PriorBasis::Fitted);
        // Six kWh on a January workday is a small household, and a *measured*
        // day is believed below the floor that only an anchor is held to.
        assert!(
            prior.kwh_per_year() < TYPICAL_HOUSE_KWH_PER_YEAR,
            "{}",
            prior.kwh_per_year()
        );
        assert!(prior.kwh_per_year() > ANCHOR_RANGE_KWH.0);

        // Nothing whole yet: no fitted scale.
        assert!(
            StandardProfile::h25(Bundesland::Be)
                .fitted(&LoadProfile::new(Bundesland::Be), at, Bundesland::Be)
                .is_none()
        );
    }

    #[test]
    fn a_learned_cell_takes_over_from_the_prior_at_four_observations() {
        // A Thursday in March: four Thursdays back are workdays in Berlin, where
        // January's would have reached Neujahr — a Sonntag by the calendar.
        let at = slot(datetime!(2026-03-12 17:45 UTC));
        let prior = StandardProfile::h25(Bundesland::Be).declared(4_000.0);
        let mut learned = LoadProfile::new(Bundesland::Be);
        assert_eq!(learned.band_with_prior(at, &prior), prior.band_at(at));
        let weeks = i64::try_from(crate::load::PRIOR_FADE_SAMPLES).expect("four");
        for week in 0..weeks {
            assert!(learned.prior_weight(at) > 0.0);
            learned.observe(
                Slot::containing(at.start() - time::Duration::weeks(week + 1)),
                Power::from_kw(2.0),
            );
        }
        assert_eq!(learned.prior_weight(at), 0.0);
        assert_eq!(learned.band_with_prior(at, &prior), learned.band_at(at));
    }

    #[test]
    fn a_declared_household_is_believed() {
        let prior = StandardProfile::h25(Bundesland::Be).declared(2_500.0);
        assert_eq!(prior.kwh_per_year(), 2_500.0);
        assert_eq!(prior.basis(), PriorBasis::Declared);
        let band = prior.band_at(slot(datetime!(2026-01-15 17:45 UTC)));
        assert!(band.p10 < band.p50 && band.p50 < band.p90);
    }
}
