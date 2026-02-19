mod server;

#[cfg(feature = "http")]
mod http_server;

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
    #[arg(long, conflicts_with = "http")]
    socket: Option<PathBuf>,

    /// Run as HTTP server on the given address (e.g. 0.0.0.0:9000)
    #[cfg(feature = "http")]
    #[arg(long)]
    http: Option<std::net::SocketAddr>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let model = match args.model {
        Some(value) => Some(value.parse::<ModelVariant>()?),
        None => None,
    };

    #[cfg(feature = "http")]
    if let Some(listen) = args.http {
        return http_server::run(http_server::HttpOptions { model, listen });
    }

    server::run(server::DaemonOptions {
        model,
        socket_path: args.socket,
    })
}
