mod link;
mod pane;
mod pty;
mod supervisor;

use std::net::SocketAddr;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tilr-server", version, about = "Terminal server: ptys behind a WebSocket")]
struct Cli {
    /// Address the WebSocket server listens on. There is no authentication
    /// yet, so keep this on loopback.
    #[arg(long, default_value = "127.0.0.1:7890")]
    listen: SocketAddr,

    /// Browser origins allowed to connect. Clients that send no Origin header
    /// (anything that is not a browser) are always allowed.
    #[arg(
        long = "allow-origin",
        default_values = ["http://127.0.0.1:7891", "http://localhost:7891"]
    )]
    allow_origins: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Internal: per-pane process started by the supervisor.
    #[command(hide = true)]
    Pane(pane::Args),
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Pane(args)) => pane::run(args),
        None => supervisor::run(cli.listen, cli.allow_origins),
    }
}
