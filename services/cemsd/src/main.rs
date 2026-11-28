mod demo;
#[cfg(feature = "opcua")]
mod opcua;
#[cfg(feature = "openadr")]
mod openadr;
mod reconcile;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(version, about = "Industrial shift peaks and interval metering")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the read-only synthetic site on loopback.
    Demo {
        #[arg(long, default_value = "127.0.0.1:8082")]
        listen: SocketAddr,
    },
    /// Rehearse a quarter-hour target with a private loopback OpenADR 3.1 VTN.
    #[cfg(feature = "openadr")]
    #[command(name = "openadr")]
    OpenAdr {
        /// Withdraw the event before the simulated battery decision.
        #[arg(long)]
        cancel_before_dispatch: bool,
        /// Print a machine-readable receipt.
        #[arg(long)]
        json: bool,
    },
    /// Capture a reviewed industrial register over one encrypted OPC UA session.
    #[cfg(feature = "opcua")]
    CaptureMeter(opcua::Capture),
    /// Compare a retained OPC UA capture with independently supplied interval closes.
    #[cfg(feature = "opcua")]
    CommissionMeter(opcua::Review),
    /// Compare one import register with closed RLM quarter hours, offline.
    Reconcile {
        trace: std::path::PathBuf,
        /// Write machine-readable JSON or a standalone offline review.
        #[arg(long, value_enum, default_value = "json")]
        format: reconcile::Format,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Demo { listen } => ems_service::edge::serve_demo(demo::router()?, listen).await,
        #[cfg(feature = "openadr")]
        Command::OpenAdr {
            cancel_before_dispatch,
            json,
        } => openadr::run(cancel_before_dispatch, json).await,
        #[cfg(feature = "opcua")]
        Command::CaptureMeter(args) => args.run().await,
        #[cfg(feature = "opcua")]
        Command::CommissionMeter(args) => args.run(),
        Command::Reconcile { trace, format } => reconcile::run(&trace, format),
    }
}
