//! Read-only mapping for the Matter Electrical Power Measurement cluster.
use matter_kit::clusters::generated::electrical_power_measurement::{ID, attribute::ACTIVE_POWER};

/// One concrete path in the Matter Server API's decimal path notation.
pub fn active_power_path(endpoint: u16) -> String {
    format!("{endpoint}/{ID}/{ACTIVE_POWER}")
}

/// ActivePower is a nullable signed number of milliwatts. Preserve export and zero.
pub fn watts(milliwatts: Option<i64>) -> Option<f64> {
    milliwatts.map(|mw| mw as f64 / 1000.0)
}
