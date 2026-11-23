//! Generate the fixed site choices from the native simulation models.
use ems_core::prelude::*;
use ems_sim::peak::{BATTERY_KW, Moment, PRIOR_KW, Trace, shift_reading};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let wallbox = Asset::Evse(Evse {
        meta: AssetMeta::new(
            AssetId::new("wallbox").expect("valid fixture ID"),
            CircuitId::new("main").expect("valid fixture ID"),
            PhaseConnection::Switchable { phase: Phase::L1 },
            Power::from_kw(11.0),
        ),
        min_current: Current::new(6.0),
        max_current: Current::new(16.0),
        bidirectional: false,
        public: false,
        charge_limit: None,
    });
    let mut rows = Vec::new();
    for watts in (0..=12_000).step_by(10) {
        let available = Power::new(f64::from(watts));
        let row = [PhaseMode::Single, PhaseMode::Three].map(|mode| {
            ems_device::realisable(&wallbox, available, mode, Envelope::at_most(available)).get()
                as u16
        });
        rows.push(row);
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(out.join("solar_setpoints.rs"), format!("{rows:?}")).expect("write solar setpoints");
    write_peak_receipts(&out);
}

fn write_peak_receipts(out: &Path) {
    let trace = Trace::bundled().expect("valid reference shift");
    let choices = [Moment::Early, Moment::Delayed].map(|moment| {
        let r = shift_reading(&trace, moment, false);
        (
            r.minutes_left,
            r.deliverable_ac_kwh.expect("complete battery evidence"),
            r.residual_peak_kw.expect("complete meter evidence"),
        )
    });
    let close = shift_reading(&trace, Moment::Closed, true);
    let kwh = close.closed_kwh.expect("reconciled closing energy");
    let kw = close.closed_mean_kw.expect("reconciled closing mean");
    let source = format!(
        "const TARGET: i32 = {PRIOR_KW};
const BATTERY: i32 = {BATTERY_KW};
const CHOICES: [(i32, f64, f64); 2] = {choices:?};
const CLOSE: (f64, f64) = ({kwh:?}, {kw:?});"
    );
    fs::write(out.join("peak_receipts.rs"), source).expect("write peak receipts");
}
