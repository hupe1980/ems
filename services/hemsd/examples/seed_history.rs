//! Create a disposable snapshot of 1,095 days of already aggregated household history.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chronix::prelude::{Chronix, ChronixConfig, FieldValue, Point, SeriesKey};
use clap::Parser;

const DAYS: i64 = 1_095;
const PER_DAY: i64 = 96;
const QUARTER_NS: i64 = 900_000_000_000;
const POINTS: [&str; 4] = ["grid", "netzwirksam", "battery", "pv"];

#[derive(Parser)]
struct Args {
    /// New, empty fixture location. An existing path is refused.
    directory: PathBuf,
    /// End of the simulated closed history (Unix seconds, on a UTC hour).
    end: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.end.rem_euclid(3_600) == 0, "end must be a UTC hour");
    std::fs::create_dir(&args.directory).context("create a new fixture directory")?;
    // Let hemsd declare the production rollup, including its wire columns and retention.
    hemsd::series::Series::open(&args.directory, 7)?.close()?;
    let mut config = ChronixConfig::small(args.directory);
    config.ooo_shard_tolerance = 0;
    config.maintenance_interval = Duration::from_secs(86_400);
    let db = Chronix::open(config)?;
    let [rollup] = <[_; 1]>::try_from(db.list_rollups()?)
        .map_err(|_| anyhow::anyhow!("expected one household rollup"))?;
    let keys = POINTS
        .map(|name| {
            SeriesKey::new(
                &rollup.target_measurement,
                BTreeMap::from([("point".to_owned(), name.to_owned())]),
            )
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let end = args
        .end
        .checked_mul(1_000_000_000)
        .context("end outside nanosecond range")?;
    let start = end
        .checked_sub(DAYS * PER_DAY * QUARTER_NS)
        .context("start outside nanosecond range")?;
    let mut written = 0;
    // Import the materialised tier itself. Building a second-by-second source trace
    // for three years would measure fixture construction instead of export.
    for day in 0..DAYS {
        let mut batch = Vec::with_capacity(384);
        for quarter in 0..PER_DAY {
            let index = day * PER_DAY + quarter;
            for (channel, key) in keys.iter().enumerate() {
                if channel == 0 && index == 2 {
                    continue; // A completely missing quarter after the spike and true zero.
                }
                let phase = i32::try_from((index + i64::try_from(channel)?) % 97)?;
                let level = f64::from(phase + 100);
                let (mean, min, max, samples) = match (channel, index) {
                    (0, 0) => (1_000., -500., 2_500., 2.),
                    (0, 1) => (0., 0., 0., 1.),
                    (3, _) => (-level, -level, -level, 1.),
                    _ => (level, level, level, 1.),
                };
                let fields = BTreeMap::from([
                    ("watts_avg".into(), FieldValue::F64(mean)),
                    ("watts_min".into(), FieldValue::F64(min)),
                    ("watts_max".into(), FieldValue::F64(max)),
                    ("watts_count".into(), FieldValue::F64(samples)),
                ]);
                batch.push(Point::new(key.clone(), fields, start + index * QUARTER_NS)?);
            }
        }
        written += db.backfill(&batch)?.into_complete()?;
        if (day + 1) % 14 == 0 {
            db.flush()?;
        }
        if (day + 1) % 365 == 0 {
            eprintln!("Imported {} of {DAYS} days", day + 1);
        }
    }
    // Advance Chronix's real watermark through the closed history. This marker
    // belongs to a separate measurement and cannot appear in a household export.
    let marker = Point::new(
        SeriesKey::new("fixture_clock", BTreeMap::new())?,
        BTreeMap::from([("tick".into(), FieldValue::I64(1))]),
        end,
    )?;
    db.insert_batch(&[marker])?.into_complete()?;
    db.flush()?;
    db.materialise_rollups()?;
    ensure!(
        db.rollup_state(&rollup.name)?.materialised_until == Some(end),
        "materialised tier did not reach the selected boundary"
    );
    ensure!(
        written == usize::try_from(DAYS * PER_DAY * 4 - 1)?,
        "unexpected fixture size"
    );
    db.close()?;
    println!("Seeded {written} materialised quarters across four points.");
    Ok(())
}
