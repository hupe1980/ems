mod charging;
mod community;
mod demo;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(version, about = "Building handover and shared connection capacity")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Replay a synthetic community allocation for one authenticated viewer.
    Community {
        #[arg(long, value_enum, default_value_t = community::Viewer::MemberA)]
        viewer: community::Viewer,
        #[arg(long, default_value = "127.0.0.1:8083")]
        listen: SocketAddr,
    },
    /// Serve the read-only synthetic site on loopback.
    Demo {
        #[arg(long, default_value = "127.0.0.1:8081")]
        listen: SocketAddr,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Community { viewer, listen } => {
            ems_service::edge::serve_demo(community::router(viewer), listen).await
        }
        Command::Demo { listen } => {
            ems_service::edge::serve_demo(demo::router(charging::Recording::load().await?), listen)
                .await
        }
    }
}
