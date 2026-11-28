//! Fixture signing and edge verification for the local canary rehearsal.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use ems_service::{Manifest, Release, UpdateError};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    Sign {
        artefact: PathBuf,
        component: String,
    },
    Verify {
        artefact: PathBuf,
        release: PathBuf,
    },
}

fn main() -> Result<()> {
    // This key identifies disposable fixtures only. Neither daemon trusts it.
    let key = SigningKey::from_bytes(&[51; 32]);
    match Cli::parse().action {
        Action::Sign {
            artefact,
            component,
        } => {
            let bytes = std::fs::read(artefact)?;
            let manifest = Manifest {
                component,
                version: "0.10.0".into(),
                url: "https://fixture.invalid/gateway".into(),
                size_bytes: u64::try_from(bytes.len())?,
                sha256: hex::encode(Sha256::digest(&bytes)),
                published_at: time::macros::datetime!(2026-01-15 06:00 UTC),
            };
            let sig = key.sign(&manifest.signing_payload()?);
            println!(
                "{}",
                serde_json::to_string(&Release {
                    manifest,
                    signature: hex::encode(sig.to_bytes())
                })?
            );
        }
        Action::Verify { artefact, release } => {
            let candidate: Release = serde_json::from_reader(std::fs::File::open(release)?)?;
            let bytes = std::fs::read(artefact).context("read simulator build")?;
            let decision =
                match candidate.verify_for("hemsd", &hex::encode(key.verifying_key().to_bytes())) {
                    Err(UpdateError::WrongComponent { .. }) => "wrong-target",
                    Err(_) => "rejected",
                    Ok(manifest)
                        if manifest.size_bytes != u64::try_from(bytes.len())?
                            || candidate.check_artefact(&bytes).is_err() =>
                    {
                        "rejected"
                    }
                    Ok(_) => "verified",
                };
            ensure!(
                !candidate.manifest.sha256.is_empty(),
                "missing candidate digest"
            );
            println!("{decision}");
        }
    }
    Ok(())
}
