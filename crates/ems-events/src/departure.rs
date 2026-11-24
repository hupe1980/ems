//! Outlet evidence from a simulated household departure.
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Whole watt-hour transaction registers from a single simulated OCPP outlet.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulatedDeparture {
    /// Time the car was due to leave the simulated site.
    #[serde(with = "time::serde::rfc3339")]
    pub departure: OffsetDateTime,
    /// Energy requested at the outlet.
    pub requested_wh: i32,
    /// Register when the transaction started.
    pub opening_wh: i32,
    /// Last register on reconnection.
    pub rejoin_wh: i32,
    /// Register observed at departure, if received.
    pub closing_wh: Option<i32>,
}

impl SimulatedDeparture {
    /// Whether the requested energy and successive register readings are admissible.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.requested_wh > 0
            && self.opening_wh >= 0
            && self.rejoin_wh >= self.opening_wh
            && self.closing_wh.is_none_or(|wh| wh >= self.rejoin_wh)
    }
}
