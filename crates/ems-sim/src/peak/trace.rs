//! Offline comparison of cumulative import and closed quarter-hour energy.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::{Duration, OffsetDateTime, UtcOffset};

/// A timestamped meter observation.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Observation instant, including its supplied offset.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// Observed energy in watt-hours.
    pub wh: u64,
}

/// Register observations and independently closed intervals over a common window.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    /// Start of the comparison window.
    #[serde(with = "time::serde::rfc3339")]
    pub start: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    end: OffsetDateTime,
    tolerance_wh: u64,
    /// Cumulative import register readings.
    pub register: Vec<Point>,
    /// Independently metered energy keyed by interval start.
    pub closed: Vec<Point>,
}

#[derive(Serialize)]
struct Interval {
    #[serde(with = "time::serde::rfc3339")]
    start: OffsetDateTime,
    register_wh: Option<u64>,
    closed_wh: Option<u64>,
    difference_wh: Option<i128>,
    status: &'static str,
}

/// Coverage and agreement between the two meter series.
#[derive(Serialize)]
pub struct Report {
    /// Whether every interval agrees within the configured tolerance.
    pub within_tolerance: bool,
    /// Intervals with both usable readings.
    pub compared_intervals: usize,
    /// Intervals that agree within tolerance.
    pub matching_intervals: usize,
    /// Largest absolute difference among compared intervals.
    pub max_absolute_difference_wh: Option<u64>,
    intervals: Vec<Interval>,
}

impl Report {
    /// Number of quarter-hour intervals in the comparison window.
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }
}

fn aligned(at: OffsetDateTime) -> bool {
    at.nanosecond() == 0 && at.unix_timestamp().rem_euclid(900) == 0
}

fn index(
    points: &[Point],
    start: OffsetDateTime,
    end: OffsetDateTime,
    closed: bool,
) -> Result<BTreeMap<OffsetDateTime, u64>> {
    let mut out = BTreeMap::new();
    for p in points {
        ensure!(
            p.at >= start && p.at <= end,
            "point outside comparison window"
        );
        ensure!(
            !closed || (aligned(p.at) && p.at < end),
            "closed intervals must use quarter-hour start instants inside the window"
        );
        if let Some(prior) = out.insert(p.at, p.wh) {
            ensure!(prior == p.wh, "conflicting duplicate at {}", p.at);
        }
    }
    Ok(out)
}

/// Reconcile each quarter against its register boundaries.
///
/// # Errors
/// Returns an error for invalid windows, conflicting duplicates or misplaced readings.
pub fn compare(t: &Trace) -> Result<Report> {
    let span = t.end - t.start;
    ensure!(
        aligned(t.start) && aligned(t.end) && span.is_positive() && span <= Duration::days(31),
        "use a positive, aligned comparison window of at most 31 days"
    );
    let register = index(&t.register, t.start, t.end, false)?;
    let closed = index(&t.closed, t.start, t.end, true)?;
    let mut intervals = Vec::new();
    let mut max_absolute_difference_wh = None;
    let mut start = t.start.to_offset(UtcOffset::UTC);
    while start < t.end {
        let end = start + Duration::minutes(15);
        let bounds = register.get(&start).zip(register.get(&end));
        let reset = !register.range(start..=end).map(|(_, wh)| wh).is_sorted();
        let register_wh = bounds.filter(|_| !reset).map(|(a, b)| b - a);
        let closed_wh = closed.get(&start).copied();
        let difference_wh = register_wh
            .zip(closed_wh)
            .map(|(a, b)| i128::from(a) - i128::from(b));
        let status = match (reset, register_wh, closed_wh) {
            (true, _, _) => "register_decreased",
            (_, None, _) => "missing_boundary",
            (_, _, None) => "missing_closed_interval",
            (_, Some(a), Some(b)) => {
                let delta = a.abs_diff(b);
                max_absolute_difference_wh =
                    Some(delta.max(max_absolute_difference_wh.unwrap_or(0)));
                if delta <= t.tolerance_wh {
                    "within_tolerance"
                } else {
                    "outside_tolerance"
                }
            }
        };
        intervals.push(Interval {
            start,
            register_wh,
            closed_wh,
            difference_wh,
            status,
        });
        start = end;
    }
    let matching_intervals = intervals
        .iter()
        .filter(|r| r.status == "within_tolerance")
        .count();
    Ok(Report {
        within_tolerance: matching_intervals == intervals.len(),
        compared_intervals: intervals
            .iter()
            .filter(|r| r.difference_wh.is_some())
            .count(),
        matching_intervals,
        max_absolute_difference_wh,
        intervals,
    })
}

impl Trace {
    /// Energy accumulated using only register evidence available at the decision.
    pub(super) fn observed_wh(&self, at: OffsetDateTime) -> Option<u64> {
        let mut samples = BTreeMap::new();
        for p in self
            .register
            .iter()
            .filter(|p| self.start <= p.at && p.at <= at)
        {
            if samples
                .insert(p.at, p.wh)
                .is_some_and(|prior| prior != p.wh)
            {
                return None;
            }
        }
        if !samples.values().is_sorted() {
            return None;
        }
        samples.get(&at)?.checked_sub(*samples.get(&self.start)?)
    }

    /// Load the synthetic reference shift.
    ///
    /// # Errors
    /// Returns an error if the bundled trace cannot be decoded.
    pub fn bundled() -> Result<Self> {
        Ok(serde_json::from_str(include_str!(
            "../../fixtures/shift-trace.json"
        ))?)
    }

    /// Reveal the independently closed first interval only after its register agrees.
    pub fn first_reconciled_wh(&self) -> Option<u64> {
        let report = compare(self).ok()?;
        let first = report.intervals.first()?;
        first
            .closed_wh
            .filter(|_| first.status == "within_tolerance")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace() -> Trace {
        serde_json::from_str(include_str!("../../fixtures/shift-trace.json")).unwrap()
    }

    #[test]
    fn repeated_hour_and_duplicates_keep_the_two_intervals_distinct() {
        let mut t = trace();
        t.register.reverse();
        t.closed.push(Point {
            at: t.start.to_offset(UtcOffset::UTC),
            wh: 285_000,
        });
        let r = compare(&t).unwrap();
        assert!(r.within_tolerance);
        assert_eq!(r.max_absolute_difference_wh, Some(1));
        assert_eq!(r.intervals.len(), 2);
        assert_eq!(r.intervals[0].difference_wh, Some(0));
        assert_eq!(r.intervals[1].difference_wh, Some(1));
    }

    #[test]
    fn missing_and_reset_intervals_never_become_zero_error() {
        let mut t = trace();
        t.tolerance_wh = 0;
        t.closed[1].wh += 2;
        let r = compare(&t).unwrap();
        assert!(!r.within_tolerance);
        assert_eq!(r.intervals[1].difference_wh, Some(-1));
        let mut t = trace();
        t.register.push(Point {
            at: t.start + Duration::minutes(3),
            wh: 5,
        });
        t.closed.pop();
        let r = compare(&t).unwrap();
        assert_eq!(r.intervals[0].status, "register_decreased");
        assert_eq!(r.intervals[1].status, "missing_closed_interval");
        assert_eq!(r.max_absolute_difference_wh, None);
        let mut t = trace();
        t.register
            .retain(|p| p.at != t.start + Duration::minutes(15));
        assert!(
            compare(&t)
                .unwrap()
                .intervals
                .iter()
                .all(|r| r.status == "missing_boundary")
        );
    }

    #[test]
    fn inconsistent_readings_and_unbounded_windows_are_refused() {
        let mut t = trace();
        t.register.push(Point { at: t.start, wh: 0 });
        assert!(compare(&t).is_err());
        let mut t = trace();
        t.end += Duration::days(32);
        assert!(compare(&t).is_err());
        let mut t = trace();
        t.closed[0].at += Duration::seconds(1);
        assert!(compare(&t).is_err());
    }
}
