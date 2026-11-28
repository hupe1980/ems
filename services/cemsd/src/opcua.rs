//! Read-only industrial meter capture and independent quarter-hour review.
use crate::reconcile::{self, ExcludedReading, Format, MAX_BYTES, MeterEvidence, read_file};
use anyhow::{Context, Result, ensure};
use clap::Args;
use ems_sim::peak::{Point, Trace};
use opcua_kit::{
    Client, ConnectionConfig,
    commissioning::{
        Contract,
        meter::{
            MeterExclusion, MeterExclusionReason, MeterPoint, project_meter_trace,
            validate_meter_contract,
        },
        parse_contract,
    },
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

fn contract(path: &Path) -> Result<Contract> {
    let contract = parse_contract(&read_file(path, MAX_BYTES)?)?;
    validate_meter_contract(&contract)?;
    Ok(contract)
}

#[derive(Args)]
pub struct Capture {
    /// Reviewed single-register contract in Wh.
    #[arg(long)]
    contract: PathBuf,
    /// Provisioned opc.tcp endpoint.
    #[arg(long)]
    endpoint: String,
    /// Pinned server leaf certificate in DER.
    #[arg(long)]
    server_cert: PathBuf,
    /// Application URI in the client certificate.
    #[arg(long)]
    client_uri: String,
    /// Client application certificate in DER.
    #[arg(long)]
    client_cert: PathBuf,
    /// Client private key in DER.
    #[arg(long)]
    client_key: PathBuf,
    /// Observations to collect before emitting NDJSON (1–4096).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=4096))]
    samples: u16,
    /// Seconds between completing a read and starting the next poll (1–900).
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u16).range(1..=900))]
    period_seconds: u16,
}

impl Capture {
    pub async fn run(self) -> Result<()> {
        let contract = contract(&self.contract)?;
        ensure!(
            contract.principal() == "anonymous",
            "capture requires a contract for the anonymous user on a certificate-authenticated session"
        );
        let credential = |path: &Path| read_file(path, 64 * 1024);
        let client = Client::connect(ConnectionConfig {
            endpoint: &self.endpoint,
            server_application_uri: contract.application_uri(),
            server_certificate_der: &credential(&self.server_cert)?,
            client_application_uri: &self.client_uri,
            client_certificate_der: &credential(&self.client_cert)?,
            client_private_key_der: &credential(&self.client_key)?,
            timeout: Duration::from_secs(5),
        })
        .context("open pinned OPC UA session")?;
        if self.samples > 1 {
            eprintln!(
                "Collecting {} readings with {} s between polls. NDJSON is written when the capture completes.",
                self.samples, self.period_seconds
            );
        }
        let capture = async {
            let mut capture = Vec::new();
            for i in 0..self.samples {
                if i > 0 {
                    tokio::time::sleep(Duration::from_secs(u64::from(self.period_seconds))).await;
                }
                let reading = client
                    .read_meter(&contract)
                    .await
                    .context("read approved import register")?;
                ensure!(
                    capture.len() + reading.compact_line_bytes() <= MAX_BYTES as usize,
                    "capture exceeds 4 MiB; use fewer samples"
                );
                serde_json::to_writer(&mut capture, &reading)?;
                capture.push(b'\n');
            }
            Ok::<_, anyhow::Error>(capture)
        }
        .await;
        client.disconnect().await;
        std::io::stdout().lock().write_all(&capture?)?;
        Ok(())
    }
}

#[derive(Args)]
pub struct Review {
    /// Reviewed contract used for capture.
    #[arg(long)]
    contract: PathBuf,
    /// Source-timed NDJSON capture from the meter.
    #[arg(long)]
    readings: PathBuf,
    /// Independent meter identity, window, closed quarters and tolerance.
    #[arg(long)]
    window: PathBuf,
    /// Emit a JSON receipt or a standalone HTML review.
    #[arg(long, value_enum, default_value = "json")]
    format: Format,
}

impl From<MeterExclusion> for ExcludedReading {
    fn from(exclusion: MeterExclusion) -> Self {
        use MeterExclusionReason as Reason;
        let (reason, status_code) = match exclusion.reason {
            Reason::Status(code) => ("status", Some(code)),
            Reason::MissingSourceTime => ("missing_source_time", None),
            Reason::SourceAfterReceipt => ("source_after_receipt", None),
            Reason::NegativeWh => ("negative_wh", None),
            Reason::FractionalWh => ("fractional_wh", None),
            Reason::NonFiniteWh => ("non_finite_wh", None),
            Reason::InsufficientResolution => ("insufficient_resolution", None),
            _ => ("other", None),
        };
        Self {
            line: exclusion.input_position,
            source_at: exclusion.source_at,
            reason,
            detail: exclusion.reason.to_string(),
            status_code,
        }
    }
}

impl Review {
    pub fn run(self) -> Result<()> {
        let contract = contract(&self.contract)?;
        let projection = project_meter_trace(
            &contract,
            &read_file(&self.readings, MAX_BYTES)?,
            &read_file(&self.window, MAX_BYTES)?,
        )
        .context("qualify register evidence against the independent meter window")?;
        let trace = projection.trace;
        let node = &contract.signals()[0].node;
        let evidence = MeterEvidence {
            device: contract.device(),
            application_uri: contract.application_uri(),
            namespace: node.namespace(),
            register: node.identifier(),
            usable_points: trace.register.len(),
            excluded_status: projection.skipped_status,
            excluded_source_time: projection.skipped_source_time,
            excluded_wh: projection.skipped_precision,
            first_exclusion: projection.first_exclusion.map(Into::into),
            outside_window: projection.outside_window,
            duplicates: projection.duplicate_readings,
        };
        let points = |rows: Vec<MeterPoint>| {
            rows.into_iter()
                .map(|p| Point {
                    at: p.at(),
                    wh: p.wh,
                })
                .collect()
        };
        let input = Trace {
            start: trace.start(),
            end: trace.end(),
            tolerance_wh: trace.tolerance_wh(),
            register: points(trace.register),
            closed: points(trace.closed),
        };
        reconcile::present(&input, self.format, Some(&evidence))
    }
}
