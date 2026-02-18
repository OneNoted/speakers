mod server;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use speake_rs_core::model::ModelVariant;

#[derive(Debug, Parser)]
#[command(
    name = "speake-rs-daemon",
    about = "Persistent Qwen3 TTS daemon for Speech Dispatcher"
)]
struct Args {
    /// Model variant to load: base or custom-voice
    #[arg(long)]
    model: Option<String>,

    /// Override daemon socket path
    #[arg(long)]
    socket: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let model = match args.model {
        Some(value) => Some(value.parse::<ModelVariant>()?),
        None => None,
    };

    server::run(server::DaemonOptions {
        model,
        socket_path: args.socket,
    })
}
