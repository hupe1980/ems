//! The German electricity day, in one place.
//!
//! A § 14a evidence day, a Modul 3 window and a daily energy figure all start at
//! 00:00 Europe/Berlin, so a summer day begins at 22:00 UTC and the days the
//! clocks change last 23 and 25 hours. `metering` owns that arithmetic
//! ([`DayBoundary::Strom`]); this module is the workspace's one door to it, so
//! the rest of hems asks for a day rather than handling the calendar's
//! supported-year range at every call site.

use metering::DayBoundary;
use time::{Date, OffsetDateTime};

pub use metering::time::calendar::to_berlin;
pub use metering::time::holiday::Bundesland;

const IN_RANGE: &str =
    "metering's calendar covers 1900–9998, which every date hems handles is inside";

/// The Berlin calendar day `instant` belongs to — not `instant.date()`, which
/// is the UTC day.
#[must_use]
pub fn local_day(instant: OffsetDateTime) -> Date {
    DayBoundary::Strom.day_of(instant).expect(IN_RANGE)
}

/// The UTC instant the Berlin day `date` starts at.
#[must_use]
pub fn day_start_utc(date: Date) -> OffsetDateTime {
    DayBoundary::Strom.day(date).expect(IN_RANGE).start()
}

/// The UTC instant the Berlin day `date` ends at (exclusive): the next day's
/// start, 23, 24 or 25 hours later.
#[must_use]
pub fn day_end_utc(date: Date) -> OffsetDateTime {
    DayBoundary::Strom.day(date).expect(IN_RANGE).end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    #[test]
    fn a_summer_day_starts_at_22_utc_and_the_change_days_are_23_and_25_hours() {
        assert_eq!(
            day_start_utc(date!(2026 - 07 - 15)),
            datetime!(2026-07-14 22:00 UTC)
        );
        assert_eq!(
            local_day(datetime!(2026-07-14 22:30 UTC)),
            date!(2026 - 07 - 15)
        );
        let spring = date!(2026 - 03 - 29);
        assert_eq!(
            (day_end_utc(spring) - day_start_utc(spring)).whole_hours(),
            23
        );
        let autumn = date!(2026 - 10 - 25);
        assert_eq!(
            (day_end_utc(autumn) - day_start_utc(autumn)).whole_hours(),
            25
        );
    }
}
