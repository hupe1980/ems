//! Native reference for the site build's WebAssembly parity check.
use ems_sim::peak::{BATTERY_KW, Moment, PRIOR_KW, Trace, shift_reading};
use serde_json::json;

fn main() {
    let trace = Trace::bundled().expect("valid bundled trace");
    let choices = [Moment::Early, Moment::Delayed].map(|moment| {
        let reading = shift_reading(&trace, moment, false);
        assert!(reading.closed_kwh.is_none() && reading.closed_mean_kw.is_none());
        json!({
            "minutes": reading.minutes_left,
            "deliveredKwh": reading.deliverable_ac_kwh,
            "projectedKw": reading.residual_peak_kw,
        })
    });
    let close = shift_reading(&trace, Moment::Closed, true);
    println!(
        "{}",
        json!({
            "targetKw": PRIOR_KW,
            "batteryKw": BATTERY_KW,
            "choices": choices,
            "closedKwh": close.closed_kwh,
            "closedKw": close.closed_mean_kw,
        })
    );
}
