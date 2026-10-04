mod clipboard;
mod fail;
mod link;
mod pane;
mod pty;
mod reload;
mod supervisor;
mod update;

use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::Duration;

use clap::{Parser, Subcommand};

/// The build: package version and commit, e.g. `0.1.0-26449dd`.
pub const VERSION: &str = env!("TILR_VERSION");
/// What the build runs on, as releases name it, e.g. `linux-aarch64`.
pub const PLATFORM: &str = env!("TILR_PLATFORM");

#[derive(Parser)]
#[command(
    name = "tilr-server",
    version = concat!(env!("TILR_VERSION"), " (", env!("TILR_PLATFORM"), ")"),
    about = "Terminal server: ptys behind a WebSocket"
)]
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

    /// Do not offer the shared clipboard: clients cannot upload content, and
    /// the panes get no `xclip` and `wl-paste` of the server's.
    #[arg(long)]
    no_clipboard: bool,

    /// Largest clipboard content a client may upload, e.g. 20MB or 512K.
    #[arg(long, default_value = "20MB", value_parser = clipboard::parse_size)]
    clipboard_max: u64,

    /// Seconds uploaded clipboard content stays available. 0 = until replaced.
    #[arg(long, default_value_t = 300)]
    clipboard_ttl: u64,

    /// Seconds a client may stay silent before it is dropped. The server
    /// pings its clients, so only one that is gone stays silent. 0 = never.
    #[arg(long, default_value_t = 30)]
    client_timeout: u64,

    /// Do not reload when the executable on disk changes. SIGUSR1 still does.
    #[arg(long)]
    no_auto_reload: bool,

    /// Internal: take over from the supervisor that exec'd this one. The
    /// value is the descriptor of its state file.
    #[arg(long, hide = true)]
    resume: Option<RawFd>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Internal: per-pane process started by the supervisor.
    #[command(hide = true)]
    Pane(pane::Args),
    /// Internal: print the versions of the reload state this executable
    /// reads. A running server asks before reloading into it.
    #[command(hide = true)]
    StateFormats,
    /// Replace this executable with the latest release. A running server
    /// reloads itself and keeps its shells.
    Update {
        /// Where releases are fetched from.
        #[arg(long, env = "TILR_CDN", default_value = update::DEFAULT_CDN)]
        cdn: String,
    },
}

/// Path this process was started from. Asked once at startup: on Linux the
/// answer goes stale as soon as the file is replaced, which is exactly when
/// a reload needs it.
pub fn exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().expect("locate own executable"))
}

fn main() -> anyhow::Result<ExitCode> {
    let exe = exe();
    let mut args = std::env::args_os();
    // Started as `xclip` or `wl-paste` through the links the server puts on
    // the PATH of its panes.
    if let Some(tool) = args.next().as_deref().and_then(clipboard::shim_name) {
        return Ok(clipboard::shim(tool, exe, args.collect()));
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Pane(args)) => pane::run(args)?,
        Some(Command::StateFormats) => print!("{}", reload::formats()),
        Some(Command::Update { cdn }) => update::run(&cdn, exe)?,
        None => supervisor::run(supervisor::Options {
            listen: cli.listen,
            allow_origins: cli.allow_origins,
            clipboard: (!cli.no_clipboard).then(|| clipboard::Limits {
                max_bytes: cli.clipboard_max,
                ttl: (cli.clipboard_ttl > 0).then(|| Duration::from_secs(cli.clipboard_ttl)),
            }),
            auto_reload: !cli.no_auto_reload,
            client_timeout: (cli.client_timeout > 0)
                .then(|| Duration::from_secs(cli.client_timeout)),
            resume: cli.resume,
        })?,
    }
    Ok(ExitCode::SUCCESS)
}
