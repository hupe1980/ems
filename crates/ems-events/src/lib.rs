//! Event type constants, CloudEvents envelopes and Standard Webhooks signatures.
//!
//! Use [`ALL`] for the catalogue. [`SITE_DAY_REPORTED`] carries household
//! KPIs to the fleet. [`SITE_DAY_CLOSED`] delivers a receipt to household
//! subscribers. [`webhook`] authenticates the original event bytes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod departure;
pub mod envelope;
pub mod webhook;

pub use envelope::{EnvelopeError, Event};
pub use webhook::{Signed, WebhookError};

// ── Grid ────────────────────────────────────────────────────────────────────

/// A network operator's § 14a limit arrived.
pub const GRID_LPC_LIMIT_RECEIVED: &str = "de.ems.grid.lpc.limit.received";
/// A § 14a limit stopped applying.
pub const GRID_LPC_LIMIT_RELEASED: &str = "de.ems.grid.lpc.limit.released";
/// The link to the Energy Guard failed and the failsafe took over.
pub const GRID_LPC_FAILSAFE_ENTERED: &str = "de.ems.grid.lpc.failsafe.entered";
/// The failsafe was released, either by contact returning or by its own timer.
pub const GRID_LPC_FAILSAFE_LEFT: &str = "de.ems.grid.lpc.failsafe.left";
/// A § 9 EEG feed-in limit arrived.
pub const GRID_LPP_LIMIT_RECEIVED: &str = "de.ems.grid.lpp.limit.received";
/// A control event was recorded for the two-year evidence log, `[A1 7.3]`.
pub const GRID_EVIDENCE_RECORDED: &str = "de.ems.grid.evidence.recorded";
/// The commanded ceiling was below the minimum power owed, `[A1 4.5]`.
pub const GRID_BELOW_MINIMUM_DETECTED: &str = "de.ems.grid.below-minimum.detected";

// ── Settlement ──────────────────────────────────────────────────────────────

/// A month's MiSpeL Abgrenzung was computed for a site, `[MiSpeL A1]`.
pub const SETTLEMENT_MISPEL_ABGRENZUNG_COMPUTED: &str =
    "de.ems.settlement.mispel.abgrenzung.computed";
/// A year's MiSpeL Pauschal figures were computed for a site, `[MiSpeL A2]`.
pub const SETTLEMENT_MISPEL_PAUSCHAL_COMPUTED: &str = "de.ems.settlement.mispel.pauschal.computed";
/// A quarter hour was allocated over an energy-sharing community, § 42c EnWG.
pub const SETTLEMENT_SHARING_ALLOCATED: &str = "de.ems.settlement.sharing.allocated";

// ── Site ────────────────────────────────────────────────────────────────────

/// A site was commissioned or re-configured.
pub const SITE_CONFIGURED: &str = "de.ems.site.configured";
/// A new plan was published by the optimiser.
pub const SITE_PLAN_PUBLISHED: &str = "de.ems.site.plan.published";
/// The optimiser could not produce a plan.
pub const SITE_PLAN_FAILED: &str = "de.ems.site.plan.failed";
/// A setpoint was issued to a device.
pub const SITE_SETPOINT_ISSUED: &str = "de.ems.site.setpoint.issued";
/// The measured grid power and the sum of the assets disagree.
pub const SITE_BALANCE_DIVERGED: &str = "de.ems.site.balance.diverged";
/// A box reported what one local day came to, `ems_core::report::DayKpis`.
///
/// Keyed by site and local day so a retransmission can update the original day.
pub const SITE_DAY_REPORTED: &str = "de.ems.site.day.reported";
/// A simulated household outlet reported departure evidence.
pub const SITE_DEPARTURE_REHEARSED: &str = "de.ems.site.departure.rehearsed";
/// A local day closed, as the household's receipt: whole cents, Wh and the
/// § 14a lines. Sent to the systems a household subscribed, not to the fleet —
/// a type of its own because its `data` is not [`SITE_DAY_REPORTED`]'s.
pub const SITE_DAY_CLOSED: &str = "de.ems.site.day.closed";
/// A goal the household set turned *tight* or *impossible*.
pub const SITE_GOAL_AT_RISK: &str = "de.ems.site.goal.at-risk";

// ── Devices ─────────────────────────────────────────────────────────────────

/// A device was discovered on the local network.
pub const DEVICE_DISCOVERED: &str = "de.ems.device.discovered";
/// A device was paired and trusted.
pub const DEVICE_PAIRED: &str = "de.ems.device.paired";
/// A device stopped reporting.
pub const DEVICE_LOST: &str = "de.ems.device.lost";
/// A device refused a command.
pub const DEVICE_COMMAND_REJECTED: &str = "de.ems.device.command.rejected";
/// A device has not done what it was asked for ten minutes.
pub const DEVICE_DEVIATION_DETECTED: &str = "de.ems.device.deviation.detected";

// ── Tests ───────────────────────────────────────────────────────────────────

/// Sent on request to check that a receiver hears the box and verifies it.
pub const TEST_PING: &str = "de.ems.test.ping";

// ── Tariffs and forecasts ───────────────────────────────────────────────────

/// New prices are available for a site.
pub const TARIFF_PUBLISHED: &str = "de.ems.tariff.published";
/// A quarter hour with a negative day-ahead price is coming (§ 51 EEG).
pub const TARIFF_NEGATIVE_PRICE_AHEAD: &str = "de.ems.tariff.negative-price.ahead";
/// A new forecast is available.
pub const FORECAST_PUBLISHED: &str = "de.ems.forecast.published";
/// A forecast failed its calibration check.
pub const FORECAST_MISCALIBRATED: &str = "de.ems.forecast.miscalibrated";

// ── Flexibility ─────────────────────────────────────────────────────────────

/// A site offered its flexibility to an aggregator.
pub const FLEX_ENVELOPE_OFFERED: &str = "de.ems.flex.envelope.offered";
/// An aggregator dispatched a site.
pub const FLEX_DISPATCH_CONFIRMED: &str = "de.ems.flex.dispatch.confirmed";

/// Every type in the catalogue.
pub const ALL: &[&str] = &[
    GRID_LPC_LIMIT_RECEIVED,
    GRID_LPC_LIMIT_RELEASED,
    GRID_LPC_FAILSAFE_ENTERED,
    GRID_LPC_FAILSAFE_LEFT,
    GRID_LPP_LIMIT_RECEIVED,
    GRID_EVIDENCE_RECORDED,
    GRID_BELOW_MINIMUM_DETECTED,
    SETTLEMENT_MISPEL_ABGRENZUNG_COMPUTED,
    SETTLEMENT_MISPEL_PAUSCHAL_COMPUTED,
    SETTLEMENT_SHARING_ALLOCATED,
    SITE_CONFIGURED,
    SITE_PLAN_PUBLISHED,
    SITE_PLAN_FAILED,
    SITE_SETPOINT_ISSUED,
    SITE_BALANCE_DIVERGED,
    SITE_DAY_REPORTED,
    SITE_DEPARTURE_REHEARSED,
    SITE_DAY_CLOSED,
    SITE_GOAL_AT_RISK,
    DEVICE_DISCOVERED,
    DEVICE_PAIRED,
    DEVICE_LOST,
    DEVICE_COMMAND_REJECTED,
    DEVICE_DEVIATION_DETECTED,
    TEST_PING,
    TARIFF_PUBLISHED,
    TARIFF_NEGATIVE_PRICE_AHEAD,
    FORECAST_PUBLISHED,
    FORECAST_MISCALIBRATED,
    FLEX_ENVELOPE_OFFERED,
    FLEX_DISPATCH_CONFIRMED,
];

/// Whether `candidate` is a catalogued event type.
#[must_use]
pub fn is_known(candidate: &str) -> bool {
    ALL.contains(&candidate)
}

/// The prefix every hems event type starts with.
pub const PREFIX: &str = "de.ems.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_namespaced_and_lower_case() {
        for t in ALL {
            assert!(t.starts_with(PREFIX), "{t} is outside the namespace");
            assert_eq!(*t, t.to_lowercase(), "{t} is not lower case");
            assert!(
                !t.contains('_'),
                "{t} uses an underscore; the separator is '-'"
            );
        }
    }

    #[test]
    fn the_catalogue_has_no_duplicates() {
        for (i, t) in ALL.iter().enumerate() {
            assert!(!ALL[..i].contains(t), "{t} appears twice");
        }
    }

    #[test]
    fn lookup_works_both_ways() {
        assert!(is_known(GRID_LPC_LIMIT_RECEIVED));
        assert!(!is_known("de.ems.grid.lpc.limit.invented"));
    }
}

/// Whether `event_type` matches a subscription `pattern`.
///
/// A glob: `*` stands for any run of characters and `?` for one. So
/// `de.ems.grid.*` wakes on every § 14a event and `*` on all of them.
///
/// The one matcher every subscription mechanism uses, for the reason [`ALL`]
/// exists: a pattern written at the place that dispatches is one nobody can
/// list, and a typo in one silently matches nothing — which looks exactly like a
/// subscriber with nothing to say.
#[must_use]
pub fn matches(pattern: &str, event_type: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Iterative rather than recursive, with one backtrack point: a pattern is
    // short and a stack overflow on a configuration string would be a poor way
    // to learn that somebody wrote `**`.
    let p: Vec<char> = pattern.chars().collect();
    let v: Vec<char> = event_type.chars().collect();
    let (mut pi, mut vi) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while vi < v.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, vi));
            pi += 1;
        } else if let Some((sp, sv)) = star {
            pi = sp + 1;
            vi = sv + 1;
            star = Some((sp, vi));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod subscription_patterns {
    use super::*;

    #[test]
    fn a_family_wakes_on_everything_under_it() {
        assert!(matches("de.ems.grid.*", GRID_LPC_LIMIT_RECEIVED));
        assert!(matches("de.ems.grid.*", GRID_BELOW_MINIMUM_DETECTED));
        assert!(!matches("de.ems.grid.*", SITE_DAY_REPORTED));
        assert!(matches("*", SITE_DAY_REPORTED));
    }

    #[test]
    fn an_exact_pattern_wakes_on_one_type() {
        assert!(matches(SITE_DAY_REPORTED, SITE_DAY_REPORTED));
        assert!(!matches(SITE_DAY_REPORTED, SITE_PLAN_PUBLISHED));
    }

    #[test]
    fn a_pattern_that_matches_nothing_in_the_catalogue_is_findable() {
        // The reason this lives beside `ALL`: a subscription nobody can list is
        // a subscriber that never runs, and that looks exactly like a
        // subscriber with nothing to say.
        assert!(
            !ALL.iter().any(|t| matches("de.ems.grid.lpc.limit", t)),
            "the family needs its wildcard, and a test says so rather than a \
             specialist silently never waking"
        );
        assert!(ALL.iter().any(|t| matches("de.ems.grid.lpc.*", t)));
    }

    #[test]
    fn a_star_in_the_middle_backtracks_correctly() {
        assert!(matches("de.*.day.reported", SITE_DAY_REPORTED));
        assert!(matches("de.ems.*.reported", SITE_DAY_REPORTED));
        assert!(!matches("de.ems.*.refused", SITE_DAY_REPORTED));
        assert!(matches("de.ems.site.day.reporte?", SITE_DAY_REPORTED));
    }
}
