//! Local trace file input and CLI reporting.
use anyhow::{Context, Result, ensure};
use ems_sim::peak::compare;
use std::{fs::File, io::Read, path::Path};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub fn run(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path).context("inspect local meter trace")?;
    ensure!(metadata.is_file(), "use a regular trace file");
    ensure!(metadata.len() <= MAX_BYTES, "trace exceeds 4 MiB");
    let file = File::open(path).context("open local meter trace")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "trace exceeds 4 MiB");
    let report = compare(&serde_json::from_slice(&bytes).context("invalid trace JSON")?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    let largest = report
        .max_absolute_difference_wh
        .map_or_else(|| "unknown".to_owned(), |wh| format!("{wh} Wh"));
    eprintln!(
        "{} of {} intervals compared, {} within tolerance. Largest difference: {largest}.",
        report.compared_intervals,
        report.interval_count(),
        report.matching_intervals
    );
    ensure!(
        report.within_tolerance,
        "incomplete or out-of-tolerance intervals; inspect the JSON report"
    );
    Ok(())
}
