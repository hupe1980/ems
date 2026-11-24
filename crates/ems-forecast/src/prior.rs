//! Cold-start demand for the uncontrolled part of a household.
//!
//! The project-owned synthetic shape combines background use, a morning peak
//! and a wider evening peak. It shifts breakfast on non-workdays and raises
//! daily energy in winter. These are modelling assumptions. The wide initial
//! band narrows as the household supplies measurements for each quarter.

use crate::load::LoadProfile;
use crate::quantile::Band;
use ems_core::calendar::{Bundesland, FIRST_YEAR, LAST_YEAR, to_berlin};
use ems_core::prelude::{DayType, Power, SLOT_HOURS, Slot};

/// Reference consumption for estimating the scale from metered demand.
const REFERENCE_KWH_PER_YEAR: f64 = 1_000.0;

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

fn local_time(slot: Slot) -> Option<time::OffsetDateTime> {
    if !(FIRST_YEAR - 1..=LAST_YEAR).contains(&slot.start().year()) {
        return None;
    }
    let local = to_berlin(slot.start());
    (FIRST_YEAR..=LAST_YEAR)
        .contains(&local.year())
        .then_some(local)
}

/// Share of annual energy assigned to a nominal 96-quarter local day.
fn daily_share(date: time::Date) -> f64 {
    let days = f64::from(time::util::days_in_year(date.year()));
    let phase = std::f64::consts::TAU * (f64::from(date.ordinal()) - 15.0) / days;
    (1.0 + 0.16 * phase.cos()) / days
}

/// Synthetic gross household demand on the site's local holiday calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseholdProfile {
    land: Bundesland,
}

impl HouseholdProfile {
    /// Construct the initial shape for a household in this federal state.
    #[must_use]
    pub fn new(land: Bundesland) -> Self {
        Self { land }
    }

    /// Quarter-hour mean demand for the given annual household consumption.
    #[must_use]
    pub fn power_at(&self, slot: Slot, kwh_per_year: f64) -> Option<Power> {
        if !kwh_per_year.is_finite() || kwh_per_year < 0.0 {
            return None;
        }
        let local = local_time(slot)?;
        let hour = f64::from(local.hour()) + f64::from(local.minute()) / 60.0 + SLOT_HOURS / 2.0;
        let breakfast = match DayType::of(slot, self.land) {
            DayType::Workday => 7.0,
            DayType::Saturday => 9.0,
            DayType::Sunday => 10.0,
        };
        let triangle = |center: f64, width: f64| (1.0 - (hour - center).abs() / width).max(0.0);
        let shape = 0.55 + 0.9 * triangle(breakfast, 3.0) + 1.9 * triangle(19.0, 4.0);
        // A unit-height triangular peak has area equal to its half-width.
        let daily_mean = 0.55 + (0.9 * 3.0 + 1.9 * 4.0) / 24.0;
        Some(Power::from_kw(
            kwh_per_year * daily_share(local.date()) / 24.0 * shape / daily_mean,
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
    /// the ratio of the household's learned energy for that day to the profile's for
    /// the nearest day of the same type up to `at`; the mean of those ratios,
    /// held to [`ANCHOR_RANGE_KWH`]. `None` until one whole day has been seen.
    #[must_use]
    pub fn fitted(self, learned: &LoadProfile, at: Slot) -> Option<Prior> {
        let mut implied = Vec::new();
        for day_type in [DayType::Workday, DayType::Saturday, DayType::Sunday] {
            let Some(own_kwh) = learned.day_kwh(day_type) else {
                continue;
            };
            let Some(reference) = self.day_kwh_like(day_type, at) else {
                continue;
            };
            if reference > 0.0 {
                implied.push(own_kwh / reference * REFERENCE_KWH_PER_YEAR);
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

    /// Match the 96 wall-clock cells summed by `LoadProfile::day_kwh`, even
    /// when the latest day of that type had a missing or repeated hour.
    fn day_kwh_like(&self, day_type: DayType, at: Slot) -> Option<f64> {
        let mut date = local_time(at)?.date();
        for _ in 0..14 {
            if date.year() < FIRST_YEAR {
                return None;
            }
            let first = Slot::containing(ems_core::calendar::day_start_utc(date));
            if DayType::of(first, self.land) == day_type {
                return Some(REFERENCE_KWH_PER_YEAR * daily_share(date));
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
            .power_at(slot, REFERENCE_KWH_PER_YEAR)
            .map_or(0.0, Power::get);
        let implied = if unit > 0.0 && reading.is_finite() {
            reading.get() / unit * REFERENCE_KWH_PER_YEAR
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

/// Gross load, scaled to one household.
#[derive(Debug, Clone, PartialEq)]
pub struct Prior {
    profile: HouseholdProfile,
    kwh_per_year: f64,
    basis: PriorBasis,
}

impl Prior {
    /// The band the prior gives `slot`: the profile's value as the median, widened by
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
    fn the_year_sums_to_the_declared_household_energy() {
        let profile = HouseholdProfile::new(Bundesland::Be);
        for (start, end) in [
            (
                datetime!(2025-12-31 23:00 UTC),
                datetime!(2026-12-31 23:00 UTC),
            ),
            (
                datetime!(2023-12-31 23:00 UTC),
                datetime!(2024-12-31 23:00 UTC),
            ),
        ] {
            let mut s = slot(start);
            let mut kwh = 0.0;
            while s.start() < end {
                kwh += profile.power_at(s, 4_000.0).unwrap().get() / 1_000.0 * SLOT_HOURS;
                s = s.next();
            }
            assert!((kwh - 4_000.0).abs() < 1.0, "{kwh} kWh");
        }
    }

    #[test]
    fn a_winter_workday_peaks_in_the_evening_and_bottoms_out_at_night() {
        // The shape the prior exists for: the night trough a cold box starts
        // from is a fraction of the evening it has to plan for.
        let profile = HouseholdProfile::new(Bundesland::Be);
        let night = profile
            .power_at(slot(datetime!(2026-01-15 02:00 UTC)), 4_000.0)
            .unwrap();
        let evening = profile
            .power_at(slot(datetime!(2026-01-15 17:45 UTC)), 4_000.0)
            .unwrap();
        assert!(
            evening.get() > 2.5 * night.get(),
            "{evening:?} against {night:?}"
        );
    }

    #[test]
    fn a_holiday_uses_the_local_sunday_shape() {
        // Fronleichnam 2026 is a Thursday: a holiday in Bayern, a workday in
        // Berlin. The same instant has two different morning peaks.
        let at = slot(datetime!(2026-06-04 10:00 UTC));
        let by = HouseholdProfile::new(Bundesland::By)
            .power_at(at, 4_000.0)
            .unwrap();
        let be = HouseholdProfile::new(Bundesland::Be)
            .power_at(at, 4_000.0)
            .unwrap();
        assert_ne!(by, be);
    }

    #[test]
    fn an_anchor_believes_a_large_household_and_never_scales_below_a_typical_house() {
        let at = slot(datetime!(2026-01-15 02:00 UTC));
        let profile = HouseholdProfile::new(Bundesland::Be);
        let large = profile.power_at(at, 6_000.0).unwrap();
        let prior = profile.clone().anchored(at, large);
        assert!((prior.kwh_per_year() - 6_000.0).abs() < 1.0);
        assert!((prior.band_at(at).p50 - large.get()).abs() < 1e-9);
        assert_eq!(prior.basis(), PriorBasis::Anchored);

        // The night trough a box usually starts from implies a small household,
        // and under-scaling is the expensive direction: held at the floor.
        let trough = profile.power_at(at, 2_000.0).unwrap();
        let held = profile.clone().anchored(at, trough);
        assert_eq!(held.kwh_per_year(), TYPICAL_HOUSE_KWH_PER_YEAR);
        let dark = profile.clone().anchored(at, Power::ZERO);
        assert_eq!(dark.kwh_per_year(), TYPICAL_HOUSE_KWH_PER_YEAR);

        // A kettle at 03:00 is not a household of thirty thousand kWh.
        let kettle = profile.anchored(at, Power::from_kw(3.0));
        assert_eq!(kettle.kwh_per_year(), ANCHOR_RANGE_KWH.1);
    }

    #[test]
    fn january_is_not_july() {
        let profile = HouseholdProfile::new(Bundesland::Be);
        let day = |at: time::OffsetDateTime| -> f64 {
            let mut s = slot(at);
            let date = s.local_date();
            let mut kwh = 0.0;
            while s.local_date() == date {
                kwh += profile.power_at(s, 4_000.0).unwrap().get() / 1_000.0 * SLOT_HOURS;
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
        let prior = HouseholdProfile::new(Bundesland::Be)
            .fitted(&learned, at)
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

        // A learned day always has 96 wall-clock cells, including when its
        // representative Sunday is a clock-change day.
        for instant in [
            datetime!(2026-03-29 12:00 UTC),
            datetime!(2026-10-25 12:00 UTC),
        ] {
            let profile = HouseholdProfile::new(Bundesland::Be);
            let at = slot(instant);
            let normal_day = profile.day_kwh_like(DayType::Sunday, at).unwrap();
            let date = at.local_date();
            let mut quarter = slot(ems_core::calendar::day_start_utc(date));
            let end = ems_core::calendar::day_end_utc(date);
            let mut energy = 0.0;
            while quarter.start() < end {
                energy += profile
                    .power_at(quarter, REFERENCE_KWH_PER_YEAR)
                    .unwrap()
                    .get()
                    * SLOT_HOURS
                    / 1_000.0;
                quarter = quarter.next();
            }
            // The clock changes during the flat overnight background load.
            let overnight_kw = profile
                .power_at(
                    slot(ems_core::calendar::day_start_utc(date)),
                    REFERENCE_KWH_PER_YEAR,
                )
                .unwrap()
                .get()
                / 1_000.0;
            let day_hours = (end - ems_core::calendar::day_start_utc(date)).whole_hours();
            let adjusted = if day_hours == 23 {
                energy + overnight_kw
            } else {
                energy - overnight_kw
            };
            assert!((normal_day - adjusted).abs() < 1e-10);
        }

        // Nothing whole yet: no fitted scale.
        assert!(
            HouseholdProfile::new(Bundesland::Be)
                .fitted(&LoadProfile::new(Bundesland::Be), at)
                .is_none()
        );
    }

    #[test]
    fn a_learned_cell_takes_over_from_the_prior_at_four_observations() {
        // A Thursday in March: four Thursdays back are workdays in Berlin, where
        // January's would have reached Neujahr — a Sonntag by the calendar.
        let at = slot(datetime!(2026-03-12 17:45 UTC));
        let prior = HouseholdProfile::new(Bundesland::Be).declared(4_000.0);
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
        let prior = HouseholdProfile::new(Bundesland::Be).declared(2_500.0);
        assert_eq!(prior.kwh_per_year(), 2_500.0);
        assert_eq!(prior.basis(), PriorBasis::Declared);
        let band = prior.band_at(slot(datetime!(2026-01-15 17:45 UTC)));
        assert!(band.p10 < band.p50 && band.p50 < band.p90);
    }
}
