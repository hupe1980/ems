//! The box's own measurement series: days of seconds, years of quarter hours.
//!
//! The guard runs once a control period against what the meters say. `[A1 7.2]`
//! asks the box to keep the minute-resolution trace during a § 14a reduction and
//! nothing more, which leaves the commonest question a household or an installer
//! has — "what was the battery doing at half past two" — unanswerable on the box
//! that watched it. This is where those readings go.
//!
//! # Two tiers, and the reader is told which
//!
//! Seconds are a *diagnostic* window measured in days; a store's worth of them
//! is rolled into quarter-hour summaries kept for three years, which is what makes
//! "this January against last January" a question the box answers without a
//! fleet behind it. [`Series::window`] picks the tier and **says which one
//! answered**: a year at one second is 31 million readings nobody asked for, and
//! a caller handed quarter-hourly means without being told would label them
//! one-second data.
//!
//! # What goes here, and what must not
//!
//! **The series, never the settlement.** What lives here is measured power — an
//! `f64` before it arrived, and diagnostic rather than statutory. The
//! quarter-hour registers, the § 14a evidence and the box's identity stay in
//! `redb` ([`crate::store`]).
//!
//! The reason is a **transaction** rather than a type. A register and the outbox
//! marker saying the fleet still owes it are written in one `redb` write
//! ([`crate::store::Store::put_quarter_hours`]); two stores have no shared
//! transaction, so splitting them would make a settlement that was stored and
//! never forwarded — or forwarded and never stored — something a power cut can
//! produce (D168).
//!
//! That split is also what makes the durability trade acceptable.
//! `ChronixConfig::small` coalesces its WAL fsync to every five seconds, so a
//! power cut can cost five seconds of the trace. Five seconds of a diagnostic
//! series is a different loss from five seconds of a legal record, and the
//! record is not here.
//!
//! # One measurement, one tag
//!
//! `power_observations{point=…}` with a single `watts` field. A tag per *point of
//! measurement* rather than a field per asset, because a household adds and
//! removes assets and a schema that grew a column for each would make the series
//! of a site that changed unreadable against the series of one that did not.
//! Cardinality is bounded by the number of assets a house has.
//!
//! The sign is `ems-core`'s throughout: **positive is drawn**, negative is fed
//! back. A series that flipped the convention per point would be a series
//! nobody can sum.

use std::collections::BTreeMap;
use std::path::Path;

use chronix::chronix_query;
use chronix::prelude::{
    Chronix, ChronixConfig, FieldValue, Parallelism, Point, RecordBatch, SeriesKey, Timestamp,
};
use ems_core::prelude::{AssetId, Power};
use time::OffsetDateTime;

/// A wall-clock instant as the store's own timestamp: nanoseconds since the
/// epoch, signed.
fn nanos(at: OffsetDateTime) -> Timestamp {
    // Saturating rather than `as`: an instant outside `i64` nanoseconds is the
    // year 2262, and a silent wrap would put a reading in 1823.
    let ns = at.unix_timestamp_nanos();
    i64::try_from(ns).unwrap_or(if ns < 0 { i64::MIN } else { i64::MAX })
}

/// The measurement every point is written under.
// Before 0.8 the `power` series sampled the registry on every control tick.
// Its rows cannot establish how many distinct observations reached the meter.
const MEASUREMENT: &str = "power_observations";

/// The tag naming the point of measurement.
const POINT: &str = "point";

/// The one field. Watts, load convention.
const WATTS: &str = "watts";

/// The measurement the quarter-hour tier is written to.
const QUARTER_HOURS: &str = "power_observations_15m_u64";

/// The name the rollup is registered under.
const ROLLUP: &str = "quarter-hours";

/// The field a quarter-hour bucket carries.
///
/// `chronix` names a rollup's output `<field>_<aggregation>`, so the mean of
/// `watts` arrives as this. Stated once, because a reader that guessed it would
/// find an empty column rather than an error.
const WATTS_MEAN: &str = "watts_avg";

/// The longest window the raw tier answers.
///
/// A day and an hour: a 25-hour Sunday in October is still one day, and asking
/// for "yesterday" in local time must not silently coarsen on the one day of the
/// year that is longer than the others.
const SECONDS_WINDOW: time::Duration = time::Duration::hours(25);

/// How long the quarter-hour tier is kept.
///
/// Three years. The raw second-by-second tier is a *diagnostic* window measured
/// in days; this is the household's own history, and three years is what makes
/// "this January against last January" a question the box can answer without a
/// fleet. It is deliberately longer than `[A1 7.3]`'s two years for the § 14a
/// record, which lives in the other store and is a different kind of thing.
const QUARTER_HOUR_YEARS: i64 = 3;

/// The point of measurement for the connection point itself.
pub const GRID: &str = "grid";

/// The netzwirksamer Leistungsbezug, `[A1 2.3]` — what a § 14a ceiling is
/// measured against, which is not the same as the grid draw.
pub const NETZWIRKSAM: &str = "netzwirksam";

/// Why the series could not be opened or written.
#[derive(Debug, thiserror::Error)]
pub enum SeriesError {
    /// The store itself.
    #[error("the measurement series failed: {0}")]
    Store(String),
    /// A tag or field name the store will not accept.
    ///
    /// An asset identifier is already narrower than `chronix`'s own rule, so
    /// this is unreachable for anything `ems-core` will construct — and it is
    /// an error rather than a panic because the alternative to recording a
    /// measurement is never a crash.
    #[error("{what} is not a name the series accepts: {detail}")]
    NotAName {
        /// Which name.
        what: String,
        /// What the store said.
        detail: String,
    },
}

/// One reading, as it comes back out.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct Reading {
    /// When it was taken.
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    pub at: OffsetDateTime,
    /// Signed watts. At quarter-hour resolution this is the arithmetic sample mean.
    pub watts: f64,
    /// Lowest observed power in the quarter, watts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_watts: Option<f64>,
    /// Highest observed power in the quarter, watts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_watts: Option<f64>,
    /// Number of observations contributing to the quarter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<u64>,
}

/// How closely spaced the readings in a window are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Individual timestamped observations.
    Seconds,
    /// The mean over a quarter hour — the grain everything in this workspace
    /// settles on, and the only one that survives past the raw retention.
    QuarterHours,
}

/// A window of history, and the grain it is in.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
#[schemars(rename = "SeriesWindow")]
pub struct Window {
    /// How closely spaced the readings are.
    pub resolution: Resolution,
    /// The readings themselves, in order.
    pub readings: Vec<Reading>,
}

/// The box's measurement series.
pub struct Series {
    db: Chronix,
    /// How long the raw tier is kept, so a read knows which tier can answer.
    raw_keeps: time::Duration,
}

impl Series {
    /// Open — or create — the series at `path`, keeping `keep_days` of it.
    ///
    /// [`ChronixConfig::small`] is the gateway profile: a memory budget of about
    /// 48 MB, a single compaction worker, and a WAL sized for flash. Retention
    /// is enforced by the store's own maintenance thread, so there is no sweep
    /// for this daemon to schedule and forget.
    ///
    /// Queries run **serially**, on the calling thread. A read that fanned out
    /// over a pool sized to the core count would do so inside the process that
    /// owes the sixty-second § 14a heartbeat, and a gateway has two to four
    /// cores. The preset already says so; it is set here as well, so the
    /// property is this daemon's statement rather than a default it inherits.
    ///
    /// # Errors
    /// [`SeriesError::Store`] where the directory cannot be opened or locked —
    /// which is what a second `hemsd` on the same box looks like.
    pub fn open(path: &Path, keep_days: u16) -> Result<Self, SeriesError> {
        let mut config = ChronixConfig::small(path);
        // Every field stays overridable after `small()`, which is what its own
        // documentation promises — so retention is set here rather than by
        // rebuilding the preset and risking a different one.
        config.retention = Some(std::time::Duration::from_secs(
            u64::from(keep_days.max(1)) * 24 * 3_600,
        ));
        config.parallelism = Parallelism::Serial;
        let db = Chronix::open(config).map_err(|e| SeriesError::Store(e.to_string()))?;
        let series = Self {
            db,
            raw_keeps: time::Duration::days(i64::from(keep_days.max(1))),
        };
        series.declare_quarter_hours()?;
        Ok(series)
    }

    /// Register the quarter-hour tier, so a household keeps three years of its
    /// own history on a box that keeps days of seconds.
    ///
    /// **Fifteen minutes, read against Europe/Berlin.** Everything this
    /// workspace settles is a `Slot` — a quarter hour of the Berlin day — and a
    /// bucket that did not line up with one would produce a history nobody could
    /// put beside a register. At this width the zone changes nothing, because
    /// Berlin's standard offset is a whole number of hours; it is stated so that
    /// the alignment is a decision rather than a coincidence, and so a coarser
    /// tier declared here later inherits the right calendar.
    ///
    /// The mean weights each observed power equally. The extrema preserve short
    /// excursions and the count reports how many observations support the mean.
    ///
    /// Materialising is the store's own — compaction runs it, and retention will
    /// not drop raw data a tier has not yet aggregated — so there is no sweep for
    /// this daemon to schedule and forget.
    fn declare_quarter_hours(&self) -> Result<(), SeriesError> {
        let wanted = chronix::RollupBuilder::new()
            .name(ROLLUP)
            .source(MEASUREMENT)
            .target(QUARTER_HOURS)
            .every("15m")
            .timezone("Europe/Berlin")
            .aggregation(chronix::RollupAggFn::Avg)
            .aggregation(chronix::RollupAggFn::Min)
            .aggregation(chronix::RollupAggFn::Max)
            .aggregation(chronix::RollupAggFn::Count)
            .group_by(POINT)
            .retention_ns(QUARTER_HOUR_YEARS * 365 * 24 * 3_600 * 1_000_000_000)
            .build()
            .map_err(|e| SeriesError::Store(e.to_string()))?;

        self.db
            .ensure_rollup(wanted)
            .map_err(|e| SeriesError::Store(e.to_string()))
    }

    /// Record simultaneous diagnostic observations.
    ///
    /// # Errors
    /// Returns an error if any observation was rejected.
    pub fn record(&self, at: OffsetDateTime, points: &[(&str, Power)]) -> Result<(), SeriesError> {
        self.record_points(points.iter().map(|&(point, power)| (point, power, at)))
    }

    /// Store driver reports at their observation times. The guard calculation
    /// has no source meter and carries the time of the control decision.
    pub(crate) fn record_events(
        &self,
        events: &[(AssetId, ems_drv::DriverEvent)],
        grid_meter: Option<&AssetId>,
        netzwirksam: Power,
        now: OffsetDateTime,
    ) -> Result<(), SeriesError> {
        let observations = events.iter().filter_map(|(asset, event)| {
            let ems_drv::DriverEvent::Measured(m) = event else {
                return None;
            };
            let power = m.fresh_power(now, crate::drivers::SILENCE)?;
            if m.at - now > crate::drivers::SILENCE {
                return None;
            }
            let point = if Some(asset) == grid_meter {
                GRID
            } else {
                asset.as_str()
            };
            Some((point, power, m.at))
        });
        self.record_points(observations.chain([(NETZWIRKSAM, netzwirksam, now)]))
    }

    fn record_points<'a>(
        &self,
        points: impl Iterator<Item = (&'a str, Power, OffsetDateTime)>,
    ) -> Result<(), SeriesError> {
        let batch = points
            .map(|(name, power, at)| Self::point(name, power, nanos(at)))
            .collect::<Result<Vec<_>, _>>()?;
        if !batch.is_empty() {
            self.db
                .insert_batch(&batch)
                .and_then(chronix::InsertResult::into_complete)
                .map_err(|e| SeriesError::Store(e.to_string()))?;
        }
        Ok(())
    }

    fn point(point: &str, power: Power, timestamp: Timestamp) -> Result<Point, SeriesError> {
        let mut tags = BTreeMap::new();
        tags.insert(POINT.to_owned(), point.to_owned());
        let key = SeriesKey::new(MEASUREMENT, tags).map_err(|e| SeriesError::NotAName {
            what: point.to_owned(),
            detail: e.to_string(),
        })?;
        let mut fields = BTreeMap::new();
        fields.insert(WATTS.to_owned(), FieldValue::F64(power.get()));
        Point::new(key, fields, timestamp).map_err(|e| SeriesError::NotAName {
            what: point.to_owned(),
            detail: e.to_string(),
        })
    }

    /// Every reading for one point of measurement in `[from, to)`, in order,
    /// and **which tier answered**.
    ///
    /// **Half-open**, like every other window in this workspace — a `Slot`,
    /// `histd`'s register window, a Nachweis. `chronix`'s own `TimeRange` is
    /// inclusive at *both* ends, so the conversion happens here, once, rather
    /// than at each caller: two windows laid end to end must not both contain
    /// the instant they meet at, and a reader who used this beside
    /// `Store::quarter_hours_between` would otherwise get a different rule from
    /// each.
    ///
    /// Returning the resolution rather than silently changing it is the point. A
    /// caller that asked for a year and got 35 040 readings has not been given
    /// second-by-second data, and a reader who was not told would draw a chart
    /// claiming it was.
    ///
    /// # Errors
    /// [`SeriesError::Store`] where the read fails.
    pub fn window(
        &self,
        point: &str,
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> Result<Window, SeriesError> {
        self.window_at_resolution(point, from, to, None)
    }

    /// Read a diagnostic window, optionally requesting quarter-hour summaries
    /// for a short interval. The raw retention and maximum raw span still apply.
    ///
    /// # Errors
    /// Returns a storage error if the query fails.
    pub fn window_at_resolution(
        &self,
        point: &str,
        from: OffsetDateTime,
        to: OffsetDateTime,
        requested: Option<Resolution>,
    ) -> Result<Window, SeriesError> {
        let resolution = if requested == Some(Resolution::QuarterHours) {
            Resolution::QuarterHours
        } else {
            self.resolution_for(from, to)
        };
        let readings = match resolution {
            Resolution::Seconds => self.raw(point, from, to)?,
            Resolution::QuarterHours => self.quarter_hours(point, from, to)?,
        };
        Ok(Window {
            resolution,
            readings,
        })
    }

    /// Which tier answers a window.
    ///
    /// Two reasons to coarsen, and either is enough.
    ///
    /// **The data is gone.** Past the raw horizon the quarter-hour tier is the
    /// only place the household's history still exists. The horizon is
    /// approached rather than exact — the store sweeps on its own schedule, so a
    /// read landing on it may find the last hours already dropped, and answering
    /// from a tier being swept underneath the reader is how a chart acquires a
    /// hole.
    ///
    /// **The answer would be unusable.** A year at one second is 31 million
    /// readings: a box with a 48 MB budget cannot assemble it, and nothing that
    /// asked for a year wanted it. [`SECONDS_WINDOW`] is where a question stops
    /// being "what was the battery doing at half past two" and starts being a
    /// history — a day, with enough slack for a 25-hour one.
    fn resolution_for(&self, from: OffsetDateTime, to: OffsetDateTime) -> Resolution {
        let age = OffsetDateTime::now_utc() - from;
        if to - from <= SECONDS_WINDOW && age < self.raw_keeps - time::Duration::hours(1) {
            Resolution::Seconds
        } else {
            Resolution::QuarterHours
        }
    }

    /// The raw tier, as it was measured.
    fn raw(
        &self,
        point: &str,
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> Result<Vec<Reading>, SeriesError> {
        let plan = self
            .db
            .query()
            .measurement(MEASUREMENT)
            .tag(POINT, point)
            // One nanosecond short of `to`, which is what makes the window
            // half-open against an inclusive range. The store's resolution *is*
            // the nanosecond, so this excludes exactly the end instant and
            // nothing else.
            .range(nanos(from), nanos(to).saturating_sub(1))
            .build()
            .map_err(|e| SeriesError::Store(e.to_string()))?;
        let batch = self
            .db
            .execute(&plan)
            .map_err(|e| SeriesError::Store(e.to_string()))?;
        Ok(readings(&batch, WATTS))
    }

    /// Read complete buckets from the materialised tier and its live tail.
    fn quarter_hours(
        &self,
        point: &str,
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> Result<Vec<Reading>, SeriesError> {
        // EMS windows exclude the end. Chronix includes both bucket starts.
        let last = nanos(to).saturating_sub(1);
        let batch = self
            .db
            .rollup_where(ROLLUP, nanos(from), last, &[(POINT, point)])
            .map_err(|e| SeriesError::Store(e.to_string()))?;
        Ok(readings(&batch, WATTS_MEAN))
    }

    /// Whether a stored quarter needs recomputing after its source data changed.
    pub(crate) fn repair_pending(&self) -> Result<bool, SeriesError> {
        self.db
            .rollup_state(ROLLUP)
            .map(|state| !state.invalid.is_empty())
            .map_err(|error| SeriesError::Store(error.to_string()))
    }

    /// Flush and release the directory lock.
    ///
    /// Called on the way out rather than left to `Drop`: what is in the memtable
    /// when a process ends is the most recent few seconds, which is the part of a
    /// diagnostic trace somebody is most likely to be looking for.
    ///
    /// # Errors
    /// [`SeriesError::Store`] where the flush fails.
    pub fn close(self) -> Result<(), SeriesError> {
        self.db
            .close()
            .map_err(|e| SeriesError::Store(e.to_string()))
    }
}

/// Turn one Arrow batch into readings.
///
/// The time column is `_time`, which is the store's own name for it everywhere —
/// a field called `timestamp` would be a second column with the same meaning.
///
/// It is read as the **`Int64`** it is stored as, and not through
/// `chronix_query::extract_f64`, which is the obvious call and is wrong here:
/// nanoseconds since the epoch is about 1,8 × 10¹⁸ in 2026, which needs 61 bits,
/// and an `f64` mantissa carries 53. Every instant would come back rounded to
/// the nearest 256 ns. Nothing in this workspace would notice at a one-second
/// cadence, which is exactly why it is worth saying: the *value* column is a
/// genuine `f64` and goes through the helper, and the time column never can.
fn readings(batch: &RecordBatch, field: &str) -> Vec<Reading> {
    use arrow::array::{Array as _, Int64Array};
    let Some(times) = batch
        .column_by_name("_time")
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
    else {
        return Vec::new();
    };
    let Some(watts) = batch.column_by_name(field) else {
        return Vec::new();
    };
    let minimum = batch.column_by_name("watts_min");
    let maximum = batch.column_by_name("watts_max");
    let count = batch
        .column_by_name("watts_count")
        .and_then(|c| c.as_any().downcast_ref::<arrow::array::UInt64Array>());
    let number = |column: Option<&arrow::array::ArrayRef>, row| {
        column.and_then(|c| chronix_query::extract_f64(c.as_ref(), row))
    };
    (0..batch.num_rows())
        .filter_map(|row| {
            let value = chronix_query::extract_f64(watts.as_ref(), row)?;
            let at =
                OffsetDateTime::from_unix_timestamp_nanos(i128::from(times.value(row))).ok()?;
            Some(Reading {
                at,
                watts: value,
                min_watts: number(minimum, row),
                max_watts: number(maximum, row),
                samples: count.filter(|c| c.is_valid(row)).map(|c| c.value(row)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of a quarter hour a few minutes ago.
    ///
    /// Relative to the clock rather than a date somebody typed: which tier
    /// answers a window depends on how old it is, so a fixed instant would have
    /// meant one thing the week it was written and another every week after.
    ///
    /// On a quarter, because a window that begins mid-bucket loses that bucket —
    /// the quarter hour it opens in started before it did, and is not the
    /// window's to report. That is right, and it is not what these tests are
    /// about.
    fn recently() -> OffsetDateTime {
        let now = OffsetDateTime::now_utc() - time::Duration::minutes(5);
        let quarter = i64::from(now.minute() % 15) * 60 + i64::from(now.second());
        (now - time::Duration::seconds(quarter))
            .replace_nanosecond(0)
            .expect("zero is a nanosecond")
    }

    /// A directory of its own, named with the process id: `cargo test` runs test
    /// binaries in parallel and `chronix` takes an exclusive lock on its own
    /// data directory, so a shared path would be one test failing on the other.
    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(what: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("hems-series-{what}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_tick_goes_in_and_comes_back_out() {
        let now = recently();
        let dir = Dir::new("roundtrip");
        let series = Series::open(&dir.0, 7).expect("a series");
        series
            .record(
                now,
                &[
                    (GRID, Power::from_kw(1.2)),
                    (NETZWIRKSAM, Power::from_kw(0.9)),
                    ("dach", Power::from_kw(-3.4)),
                ],
            )
            .expect("one tick");

        let grid = series
            .window(
                GRID,
                now - time::Duration::minutes(1),
                now + time::Duration::minutes(1),
            )
            .expect("the grid series")
            .readings;
        assert_eq!(grid.len(), 1, "one reading, not one per point: {grid:?}");
        assert!((grid[0].watts - 1_200.0).abs() < 1e-6, "{:?}", grid[0]);
        assert_eq!(grid[0].at, now, "to the instant it was taken");

        // The sign convention survives, which is the half a series that mixes
        // producers and consumers has to get right: a roof feeding back is
        // negative in `ems-core`'s load convention and must not come back
        // positive.
        let pv = series
            .window(
                "dach",
                now - time::Duration::minutes(1),
                now + time::Duration::minutes(1),
            )
            .expect("the roof")
            .readings;
        assert!((pv[0].watts + 3_400.0).abs() < 1e-6, "{:?}", pv[0]);

        series.close().expect("a clean close");
    }

    #[test]
    fn one_point_of_measurement_does_not_return_anothers() {
        let now = recently();
        // The tag is the whole of the separation, so this is what says the tag
        // is being applied at all — a query that ignored it would return three
        // readings and every figure drawn from the series would be a mixture.
        let dir = Dir::new("tags");
        let series = Series::open(&dir.0, 7).expect("a series");
        for minute in 0..3 {
            let at = now + time::Duration::minutes(minute);
            series
                .record(
                    at,
                    &[(GRID, Power::from_kw(1.0)), ("dach", Power::from_kw(-2.0))],
                )
                .expect("a tick");
        }
        let window = (
            now - time::Duration::hours(1),
            now + time::Duration::hours(1),
        );
        let count = |point: &str| {
            series
                .window(point, window.0, window.1)
                .expect("a window")
                .readings
                .len()
        };
        assert_eq!(count(GRID), 3);
        assert_eq!(count("dach"), 3);
        assert_eq!(
            count("nothing"),
            0,
            "a point nobody measured has no readings"
        );
        series.close().expect("a clean close");
    }

    #[test]
    fn a_window_returns_only_what_is_inside_it() {
        let now = recently();
        let dir = Dir::new("window");
        let series = Series::open(&dir.0, 7).expect("a series");
        for minute in 0..10 {
            series
                .record(
                    now + time::Duration::minutes(minute),
                    &[(GRID, Power::from_kw(minute as f64))],
                )
                .expect("a tick");
        }
        let inside = series
            .window(
                GRID,
                now + time::Duration::minutes(2),
                now + time::Duration::minutes(5),
            )
            .expect("a window")
            .readings;
        let watts: Vec<f64> = inside.iter().map(|r| r.watts).collect();
        assert_eq!(
            watts,
            vec![2_000.0, 3_000.0, 4_000.0],
            "half-open: the start is in and the end is not"
        );
        series.close().expect("a clean close");
    }

    #[test]
    fn a_short_recent_window_comes_back_as_it_was_measured() {
        let now = recently();
        let dir = Dir::new("tier-seconds");
        let series = Series::open(&dir.0, 7).expect("a series");
        series
            .record(now, &[(GRID, Power::from_kw(1.0))])
            .expect("a tick");
        let window = series
            .window(
                GRID,
                now - time::Duration::hours(1),
                now + time::Duration::minutes(1),
            )
            .expect("a window");
        assert_eq!(window.resolution, Resolution::Seconds);
        assert_eq!(window.readings.len(), 1);
        series.close().expect("a clean close");
    }

    #[test]
    fn a_long_window_is_answered_at_quarter_hours_and_says_so() {
        // Thirty hours of a household, written forwards — the store's live path
        // accepts a write only within a couple of shards of the newest one, so a
        // test that started at the end and worked backwards would be writing
        // into a window that had closed.
        //
        // What this pins is the whole tier: that the rollup was declared, that
        // the store aggregates it, that the view answers above the watermark as
        // well as below it, and that a quarter hour comes back as the *mean* of
        // what was in it rather than a sum or a sample.
        let now = recently();
        let dir = Dir::new("tier-quarters");
        let series = Series::open(&dir.0, 7).expect("a series");

        let start = now - time::Duration::hours(30);
        // Two readings in each quarter hour, 1 kW apart, so the mean is a figure
        // neither of them is — and the roof carries the **negative** of the
        // connection point's, so the two series are told apart by value and not
        // only by how many came back. Writing both the same made a filter that
        // returned the wrong point indistinguishable from one that worked.
        for quarter in 0..120_i64 {
            let at = start + time::Duration::minutes(quarter * 15);
            let low = Power::from_kw(quarter as f64);
            let high = Power::from_kw(quarter as f64 + 1.0);
            series
                .record(at, &[(GRID, low), ("dach", -low)])
                .expect("a tick");
            series
                .record(
                    at + time::Duration::minutes(7),
                    &[(GRID, high), ("dach", -high)],
                )
                .expect("a tick");
        }

        let window = series
            .window(GRID, start, now + time::Duration::minutes(1))
            .expect("a window");
        assert_eq!(
            window.resolution,
            Resolution::QuarterHours,
            "thirty hours is not a question anybody wants 108 000 readings to"
        );
        assert_eq!(
            window.readings.len(),
            120,
            "one reading per quarter hour of the window: {:?}",
            window.readings.first().zip(window.readings.last())
        );
        let first = window.readings[0];
        assert!(
            (first.watts - 500.0).abs() < 1e-6,
            "the mean of 0 kW and 1 kW, not either of them: {first:?}"
        );
        assert_eq!(first.at, start, "the bucket the readings fell in");
        // The last quarter hour is above the materialisation watermark — the
        // store aggregates only what can no longer change — so its presence is
        // what says the live half of the view is spliced on.
        let last = window.readings[119];
        assert!(
            (last.watts - 119_500.0).abs() < 1e-6,
            "the most recent bucket, still being written: {last:?}"
        );

        // And the tag survives the coarsening. The filter is the **store's**
        // since 0.6 (`rollup_where`), which is the reason to check it by value
        // here rather than trust it: a filter that did nothing would return both
        // points and double the count, and one that returned the wrong point
        // would keep the count and flip the sign.
        let roof = series
            .window("dach", start, now + time::Duration::minutes(1))
            .expect("a window");
        assert_eq!(roof.readings.len(), 120);
        assert!(
            (roof.readings[0].watts + 500.0).abs() < 1e-6,
            "the roof's own mean, not the connection point's: {:?}",
            roof.readings[0]
        );
        assert!(
            (roof.readings[119].watts + 119_500.0).abs() < 1e-6,
            "…above the watermark too: {:?}",
            roof.readings[119]
        );
        assert!(
            series
                .window("nothing", start, now + time::Duration::minutes(1))
                .expect("a window")
                .readings
                .is_empty(),
            "a point nobody measured has no quarter hours either"
        );

        series.close().expect("a clean close");
    }

    #[test]
    fn the_stored_half_of_the_tier_says_what_the_live_half_did() {
        // The two halves of the view answer from different places — the target
        // measurement below the materialisation watermark, the raw data above it
        // — and a household reading its own January is reading the first. In a
        // test nothing has been materialised yet, so every other case here is
        // exercising the live half only; this one runs the store's own
        // materialisation and asks the same question again.
        let now = recently();
        let dir = Dir::new("tier-materialised");
        let series = Series::open(&dir.0, 7).expect("a series");
        let start = now - time::Duration::hours(30);
        for quarter in 0..120_i64 {
            let at = start + time::Duration::minutes(quarter * 15);
            series
                .record(at, &[(GRID, Power::from_kw(quarter as f64))])
                .expect("a tick");
            series
                .record(
                    at + time::Duration::minutes(7),
                    &[(GRID, Power::from_kw(quarter as f64 + 1.0))],
                )
                .expect("a tick");
        }
        let before = series.window(GRID, start, now).expect("a window");

        let oldest = before.readings[0];
        assert_eq!(
            (oldest.min_watts, oldest.max_watts, oldest.samples),
            (Some(0.), Some(1000.), Some(2))
        );
        let first_bucket = || {
            series
                .window_at_resolution(
                    GRID,
                    start,
                    start + time::Duration::seconds(1),
                    Some(Resolution::QuarterHours),
                )
                .expect("one quarter-hour start")
                .readings
        };
        assert_eq!(first_bucket(), vec![oldest]);
        let empty_schema = || {
            let read = |point| {
                series
                    .db
                    .rollup_where(ROLLUP, nanos(start), nanos(start), &[(POINT, point)])
                    .unwrap()
            };
            let empty = read("absent");
            assert_eq!(empty.num_rows(), 0);
            let populated = read(GRID);
            assert_eq!(populated.num_rows(), 1);
            assert_eq!(empty.schema(), populated.schema());
            empty.schema()
        };
        let schema = empty_schema();
        let written = series
            .db
            .materialise_rollups()
            .expect("the store aggregates what can no longer change");
        assert!(
            written > 0,
            "nothing was materialised, so this proves nothing about the stored half"
        );

        assert_eq!(empty_schema(), schema);
        assert_eq!(first_bucket(), vec![oldest]);
        let after = series.window(GRID, start, now).expect("a window");
        assert_eq!(
            after.readings, before.readings,
            "the stored half and the live half disagree about the same quarter hours"
        );
        assert!(
            !series.repair_pending().unwrap(),
            "the live tail is healthy"
        );
        let late = Series::point(
            GRID,
            Power::new(2000.),
            nanos(start + time::Duration::minutes(10)),
        )
        .unwrap();
        series
            .db
            .backfill(&[late])
            .unwrap()
            .into_complete()
            .unwrap();
        assert!(series.repair_pending().unwrap());
        assert!(series.db.materialise_rollups().unwrap() > 0);
        assert!(!series.repair_pending().unwrap());
        let repaired = first_bucket();
        assert_eq!(repaired.len(), 1);
        assert_eq!((repaired[0].samples, repaired[0].watts), (Some(3), 1000.));
        series.close().expect("a clean close");
    }

    #[test]
    fn a_quarter_hour_bucket_starts_where_a_settlement_slot_does() {
        // Every quarter hour this workspace settles is a `Slot` of the
        // Europe/Berlin day. A tier whose buckets were offset from those would
        // produce a history nobody could put beside a register — and the two
        // agree only because Berlin's offset from UTC is a whole number of
        // hours, which is a fact about the zone rather than about the store.
        let now = recently();
        let dir = Dir::new("tier-alignment");
        let series = Series::open(&dir.0, 7).expect("a series");
        let start = now - time::Duration::hours(30);
        for minute in 0..(31 * 60_i64) {
            series
                .record(
                    start + time::Duration::minutes(minute),
                    &[(GRID, Power::from_kw(1.0))],
                )
                .expect("a tick");
        }
        let window = series.window(GRID, start, now).expect("a window");
        for reading in &window.readings {
            let berlin = reading
                .at
                .to_offset(time::UtcOffset::from_hms(1, 0, 0).expect("a whole hour"));
            assert_eq!(
                (berlin.minute() % 15, berlin.second()),
                (0, 0),
                "a bucket that is not on a quarter of the Berlin hour: {reading:?}"
            );
        }
        series.close().expect("a clean close");
    }

    #[test]
    fn the_second_run_of_the_box_finds_its_own_tier() {
        // The declaration is persisted, so every run after the first meets a
        // rollup that already exists. A box that treated that as an error would
        // fail to start on its second boot — and one that declared a *second*
        // tier would aggregate everything twice.
        let dir = Dir::new("tier-reopen");
        let series = Series::open(&dir.0, 7).expect("a first run");
        series.close().expect("a clean close");

        let again = Series::open(&dir.0, 7).expect("a second run");
        let declared = again.db.list_rollups().expect("the registry");
        assert_eq!(declared.len(), 1, "{declared:?}");
        assert_eq!(declared[0].target_measurement, QUARTER_HOURS);
        again.close().expect("a clean close");
    }

    #[test]
    fn conflicting_tier_blocks_startup() {
        // What an upgrade looks like: a box in the field carries the tier an
        // older version asked for, and the registry outlives the binary. Without
        // the comparison it would keep aggregating to the old bucket for three
        // years and nothing would say so.
        let dir = Dir::new("tier-upgrade");
        {
            let series = Series::open(&dir.0, 7).expect("a series");
            series.db.delete_rollup(ROLLUP).expect("the tier");
            let stale = chronix::RollupBuilder::new()
                .name(ROLLUP)
                .source(MEASUREMENT)
                .target(QUARTER_HOURS)
                .every("1h")
                .aggregation(chronix::RollupAggFn::Avg)
                .group_by(POINT)
                .build()
                .expect("a rollup an older version might have wanted");
            series.db.create_rollup(stale).expect("declared");
            series.close().expect("a clean close");
        }

        assert!(matches!(
            Series::open(&dir.0, 7),
            Err(SeriesError::Store(_))
        ));
    }

    #[test]
    fn legacy_tick_history_requires_a_fresh_tier() {
        let dir = Dir::new("legacy-observations");
        let now = recently();
        let old = now - time::Duration::hours(30);
        let db = Chronix::open(ChronixConfig::small(&dir.0)).unwrap();
        let legacy = chronix::RollupBuilder::new()
            .name("quarter-hours")
            .source("power")
            .target("power_15m")
            .every("15m")
            .timezone("Europe/Berlin")
            .aggregation(chronix::RollupAggFn::Avg)
            .group_by(POINT)
            .build()
            .unwrap();
        db.create_rollup(legacy).unwrap();
        let key = SeriesKey::new("power", BTreeMap::from([(POINT.into(), GRID.into())])).unwrap();
        for at in [old, now] {
            db.insert(
                &Point::new(
                    key.clone(),
                    BTreeMap::from([(WATTS.into(), FieldValue::F64(100.))]),
                    nanos(at),
                )
                .unwrap(),
            )
            .unwrap();
        }
        assert!(db.materialise_rollups().unwrap() > 0);
        db.close().unwrap();

        assert!(
            matches!(Series::open(&dir.0, 7), Err(SeriesError::Store(message)) if message.contains("target_measurement"))
        );
    }

    #[test]
    fn a_spike_survives_a_control_tick_and_a_cached_reading_leaves_a_gap() {
        use ems_core::prelude::Measurement;
        use ems_drv::DriverEvent;

        let dir = Dir::new("sample-count");
        let series = Series::open(&dir.0, 7).unwrap();
        let start = recently() - time::Duration::hours(1);
        let meter = AssetId::new("meter").unwrap();
        let event = |at, watts| {
            (
                meter.clone(),
                DriverEvent::Measured(Measurement::power(at, Power::new(watts))),
            )
        };
        let first = start + time::Duration::seconds(1);
        let peak = first + time::Duration::milliseconds(100);
        let later = start + time::Duration::minutes(30);
        let still_idle = later + time::Duration::milliseconds(100);
        series
            .record_events(
                &[event(first, -500.), event(peak, 900.)],
                Some(&meter),
                Power::ZERO,
                peak,
            )
            .unwrap();
        series
            .record_events(
                &[event(peak, 900.)],
                Some(&meter),
                Power::ZERO,
                peak + time::Duration::seconds(1),
            )
            .unwrap();
        series
            .record_events(
                &[],
                Some(&meter),
                Power::ZERO,
                start + time::Duration::minutes(15),
            )
            .unwrap();
        series
            .record_events(
                &[
                    event(start + time::Duration::minutes(15), 8000.),
                    event(later, 0.),
                    event(still_idle, 0.),
                ],
                Some(&meter),
                Power::ZERO,
                still_idle,
            )
            .unwrap();

        let end = later + time::Duration::seconds(1);
        let summaries = series
            .window_at_resolution(GRID, start, end, Some(Resolution::QuarterHours))
            .unwrap()
            .readings;
        assert_eq!(summaries.len(), 2);
        assert_eq!(
            summaries[0],
            Reading {
                at: start,
                watts: 200.,
                min_watts: Some(-500.),
                max_watts: Some(900.),
                samples: Some(2)
            }
        );
        assert_eq!(summaries[1].at, later);
        assert_eq!(summaries[1].samples, Some(2));
        assert_eq!(summaries[1].watts, 0.);
        let raw = series.window(GRID, start, end).unwrap().readings;
        assert_eq!(raw.len(), 4);
        assert_eq!(raw[1].at, peak);
        assert_eq!(
            serde_json::to_value(raw[1])
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            2
        );
        series.close().unwrap();
    }
}
