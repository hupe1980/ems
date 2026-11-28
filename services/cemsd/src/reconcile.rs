//! Local trace file input and CLI reporting.
mod html;

use anyhow::{Context, Result, ensure};
use ems_sim::peak::{Trace, compare};
use serde::Serialize;
use std::{fs::File, io::Read, path::Path};

pub(super) const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Format {
    Json,
    Html,
}

pub(super) fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    ensure!(metadata.is_file(), "use a regular input file");
    ensure!(metadata.len() <= limit, "input exceeds {limit} bytes");
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "input exceeds {limit} bytes");
    Ok(bytes)
}

#[derive(Serialize)]
pub(super) struct ExcludedReading {
    pub line: usize,
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_at: Option<time::OffsetDateTime>,
    pub reason: &'static str,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct MeterEvidence<'a> {
    pub device: &'a str,
    pub application_uri: &'a str,
    pub namespace: &'a str,
    pub register: &'a str,
    pub usable_points: usize,
    pub excluded_status: usize,
    pub excluded_source_time: usize,
    pub excluded_wh: usize,
    pub first_exclusion: Option<ExcludedReading>,
    pub outside_window: usize,
    pub duplicates: usize,
}

pub fn run(path: &Path, format: Format) -> Result<()> {
    let bytes = read_file(path, MAX_BYTES)?;
    let trace = serde_json::from_slice(&bytes).context("invalid trace JSON")?;
    present(&trace, format, None)
}

pub(super) fn present(
    trace: &Trace,
    format: Format,
    evidence: Option<&MeterEvidence<'_>>,
) -> Result<()> {
    let report = compare(trace)?;
    let output = match format {
        Format::Json => match evidence {
            Some(meter) => {
                #[derive(Serialize)]
                struct Review<'a> {
                    meter: &'a MeterEvidence<'a>,
                    tolerance_wh: u64,
                    reconciliation: &'a ems_sim::peak::Report,
                }
                serde_json::to_string_pretty(&Review {
                    meter,
                    tolerance_wh: trace.tolerance_wh,
                    reconciliation: &report,
                })?
            }
            None => serde_json::to_string_pretty(&report)?,
        },
        Format::Html => html::render(&report, trace.tolerance_wh, evidence)?,
    };
    use std::io::Write;
    writeln!(std::io::stdout().lock(), "{output}")?;
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
        "incomplete or out-of-tolerance intervals; inspect the interval report"
    );
    Ok(())
}
