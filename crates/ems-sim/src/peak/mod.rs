//! Read-only rehearsal of the first quarter in the bundled register trace.
mod trace;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
pub use trace::{Interval, IntervalStatus, Point, Report, Trace, compare};

/// Prior demand peak used as the shift's target quarter-hour mean, in kW.
pub const PRIOR_KW: i32 = 1100;
/// Maximum AC discharge power of the example battery, in kW.
pub const BATTERY_KW: i32 = 120;
const REMAINING_LOAD_KW: i32 = 900;
const USABLE_KWH: i32 = 10;

/// Observation time in the first quarter hour of the bundled shift.
#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Moment {
    /// Five minutes remain.
    #[default]
    Early,
    /// Three minutes remain.
    Delayed,
    /// Accumulated import already exceeds the target.
    Unavoidable,
    /// End of the quarter.
    Closed,
}

impl Moment {
    const fn minutes_left(self) -> i32 {
        match self {
            Self::Early => 5,
            Self::Delayed | Self::Unavoidable => 3,
            Self::Closed => 0,
        }
    }
    /// Observation instant, including its supplied offset.
    pub fn at(self, start: OffsetDateTime) -> OffsetDateTime {
        start + Duration::minutes(i64::from(15 - self.minutes_left()))
    }
}

/// Evidence available at the observation instant.
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// All reference observations are available.
    #[default]
    Complete,
    /// Current register observation unavailable.
    StaleMeter,
    /// Battery power unavailable.
    BatteryUnknown,
    /// Tariff under review.
    TariffReview,
    /// Discharge power limited to 60 kW.
    LimitedBattery,
    /// Independent meter close unavailable.
    MissingClose,
}

impl Evidence {
    const fn battery_kw(self) -> Option<i32> {
        match self {
            Self::BatteryUnknown => None,
            Self::LimitedBattery => Some(60),
            _ => Some(BATTERY_KW),
        }
    }
}

/// Observation and evidence to use for one replay.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Select {
    /// Selected instant in the reference quarter.
    pub moment: Moment,
    /// Availability of simulated evidence.
    pub evidence: Evidence,
    /// Include the independent meter close when available.
    pub reveal: bool,
}

/// Projected dispatch outcome and, when revealed, the reconciled meter close.
#[derive(Serialize)]
pub struct Reading {
    /// Observation instant, including its supplied offset.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// Identifies a synthetic scenario.
    pub simulated: bool,
    /// Availability of simulated evidence.
    pub evidence: Evidence,
    /// Previous demand peak in kW.
    pub prior_peak_kw: i32,
    /// Whole minutes remaining in the quarter.
    pub minutes_left: i32,
    /// Energy already imported in this quarter.
    pub import_kwh: Option<f64>,
    /// Mean demand already implied by accumulated import.
    pub minimum_mean_kw: Option<f64>,
    /// Battery energy required to reach the target.
    pub required_ac_kwh: Option<f64>,
    /// Unassisted projected mean in kW.
    pub projected_kw: Option<f64>,
    /// Average discharge power required for the target.
    pub required_kw: Option<f64>,
    /// Average discharge power available for this decision.
    pub achievable_kw: Option<f64>,
    /// AC energy supplied by the simulated dispatch.
    pub deliverable_ac_kwh: Option<f64>,
    /// Projected quarter-hour demand after dispatch.
    pub residual_peak_kw: Option<f64>,
    /// Potential annual demand cost without dispatch.
    pub unassisted_exposure_eur: Option<i64>,
    /// Potential annual demand cost after dispatch.
    pub residual_exposure_eur: Option<i64>,
    /// Longest whole-minute delay that can still meet the target.
    pub latest_feasible_delay_min: Option<i32>,
    /// Projected demand if dispatch starts one minute too late.
    pub next_minute_mean_kw: Option<f64>,
    /// Whether the original close was requested or reached.
    pub revealed: bool,
    /// Timestamp of the reconciled close.
    #[serde(with = "time::serde::rfc3339::option")]
    pub closed_at: Option<OffsetDateTime>,
    /// Original closed energy in kWh.
    pub closed_kwh: Option<f64>,
    /// Original closed quarter-hour mean in kW.
    pub closed_mean_kw: Option<f64>,
}

fn numeric(n: Option<Decimal>) -> Option<f64> {
    n.and_then(|value| value.to_f64())
}

// The example tariff expresses potential annual exposure in whole euros.
fn exposure(evidence: Evidence, mean_kw: Option<Decimal>) -> Option<i64> {
    if evidence == Evidence::TariffReview {
        return None;
    }
    let delta = mean_kw? - Decimal::from(PRIOR_KW);
    (delta.max(Decimal::ZERO) * Decimal::from(100))
        .round()
        .to_i64()
}

fn deliverable(limit_kw: i32, minutes: i32) -> Decimal {
    (Decimal::from(limit_kw * minutes) / Decimal::from(60)).min(Decimal::from(USABLE_KWH))
}

/// Replay the complete-evidence scenario used by the browser challenge.
pub fn shift_reading(trace: &Trace, moment: Moment, reveal: bool) -> Reading {
    replay(
        trace,
        &Select {
            moment,
            reveal,
            ..Select::default()
        },
    )
}

/// Evaluate a selected observation against the previous demand peak.
pub fn replay(t: &Trace, q: &Select) -> Reading {
    replay_for_limit(t, q, PRIOR_KW)
}

/// Project the residual mean for an externally supplied target in kW.
pub fn projected_mean(t: &Trace, moment: Moment, requested_kw: i32) -> Option<f64> {
    let select = Select {
        moment,
        ..Select::default()
    };
    replay_for_limit(t, &select, requested_kw).residual_peak_kw
}

fn replay_for_limit(t: &Trace, q: &Select, requested_kw: i32) -> Reading {
    let minutes = q.moment.minutes_left();
    let at = q.moment.at(t.start);
    let register = |at| t.register.iter().find(|p| p.at == at);
    let sample = register(at);
    // At the clock change the closing sample carries the new Berlin offset.
    let at = sample.map_or(at, |p| p.at);
    let import = t
        .observed_wh(at)
        .map(|wh| Decimal::from(wh) / Decimal::from(1000))
        .map(|kwh| {
            if q.moment == Moment::Unavoidable {
                kwh + Decimal::from(40)
            } else {
                kwh
            }
        })
        .filter(|_| q.evidence != Evidence::StaleMeter);
    let quarter = Decimal::from(4);
    let minimum = import.map(|kwh| kwh * quarter);
    let projected_energy = import
        .filter(|_| minutes > 0)
        .map(|kwh| kwh + Decimal::from(REMAINING_LOAD_KW * minutes) / Decimal::from(60));
    let target = Decimal::from(requested_kw) / quarter;
    let required_energy = projected_energy
        .filter(|_| minimum.is_some_and(|mean| mean <= Decimal::from(requested_kw)))
        .map(|kwh| (kwh - target).max(Decimal::ZERO));
    let required = required_energy.map(|kwh| kwh * Decimal::from(60) / Decimal::from(minutes));
    let limit = q.evidence.battery_kw();
    let delivered = required_energy
        .zip(limit)
        .map(|(need, kw)| need.min(deliverable(kw, minutes)));
    let achievable = delivered.map(|kwh| kwh * Decimal::from(60) / Decimal::from(minutes));
    let projected = projected_energy.map(|kwh| kwh * quarter);
    let residual = projected_energy
        .zip(delivered)
        .map(|(base, kwh)| (base - kwh) * quarter);
    let latest = required_energy
        .filter(|need| *need > Decimal::ZERO)
        .zip(limit)
        .and_then(|(need, kw)| {
            (0..minutes)
                .rev()
                .find(|&delay| deliverable(kw, minutes - delay) >= need)
        });
    let next_minute = latest
        .zip(projected_energy)
        .zip(limit)
        .map(|((delay, base), kw)| (base - deliverable(kw, minutes - delay - 1)) * quarter);
    let revealed = q.reveal || q.moment == Moment::Closed;
    let closed = (revealed && !matches!(q.evidence, Evidence::MissingClose | Evidence::StaleMeter))
        .then(|| t.first_reconciled_wh())
        .flatten()
        .map(|wh| Decimal::from(wh) / Decimal::from(1000));
    Reading {
        at,
        simulated: true,
        evidence: q.evidence,
        prior_peak_kw: PRIOR_KW,
        minutes_left: minutes,
        import_kwh: numeric(import),
        minimum_mean_kw: numeric(minimum),
        required_ac_kwh: numeric(required_energy),
        projected_kw: numeric(projected),
        required_kw: numeric(required),
        achievable_kw: numeric(achievable),
        deliverable_ac_kwh: numeric(delivered),
        residual_peak_kw: numeric(residual),
        unassisted_exposure_eur: exposure(q.evidence, projected),
        residual_exposure_eur: exposure(q.evidence, residual),
        latest_feasible_delay_min: latest,
        next_minute_mean_kw: numeric(next_minute),
        revealed,
        closed_at: closed
            .and_then(|_| register(t.start + Duration::minutes(15)).map(|point| point.at)),
        closed_kwh: numeric(closed),
        closed_mean_kw: numeric(closed.map(|kwh| kwh * quarter)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_uses_only_consistent_register_evidence_available_at_decision_time() {
        let mut trace = Trace::bundled().unwrap();
        let at = Moment::Early.at(trace.start);
        let project = |trace: &Trace| shift_reading(trace, Moment::Early, false).residual_peak_kw;
        trace.register.reverse();
        let wh = trace.register.iter().find(|p| p.at == at).unwrap().wh;
        trace.register.push(Point { at, wh });
        trace.register.push(Point {
            at: at + Duration::minutes(1),
            wh: 0,
        });
        assert_eq!(project(&trace), Some(1100.0));
        trace.register.push(Point { at, wh: wh + 1 });
        assert_eq!(project(&trace), None);
        trace.register.pop();
        trace.register.push(Point {
            at: at - Duration::minutes(1),
            wh: 0,
        });
        assert_eq!(project(&trace), None);
    }
}
