mod auth;
mod clipboard;
mod cloud;
mod config;
mod fail;
mod files;
mod host;
mod identity;
mod link;
mod pane;
mod ps;
mod pty;
mod reload;
mod socks;
mod supervisor;
mod update;

use std::net::SocketAddr;
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
    /// Address the WebSocket server listens on.
    #[arg(long, default_value = "127.0.0.1:7890")]
    listen: SocketAddr,

    /// Browser origins allowed to connect. Clients that send no Origin header
    /// (anything that is not a browser) are always allowed. Given, replaces
    /// the configuration file's list; neither: tilr://app, tilr://tilr,
    /// http://127.0.0.1:7891, http://localhost:7891.
    #[arg(long = "allow-origin")]
    allow_origins: Vec<String>,

    /// The configuration file, see config.rs. Default:
    /// $XDG_CONFIG_HOME/tilr/server.toml (~/.config/tilr/server.toml), which
    /// may be missing; a file named here must exist.
    #[arg(long, env = "TILR_CONFIG")]
    config: Option<PathBuf>,

    /// Let anyone who can reach the server use it, without the access token.
    /// A server started this way gets no tunnel from tilr Cloud.
    #[arg(long)]
    no_auth: bool,

    /// The `cloudflared` that carries the tunnel of a server linked to tilr
    /// Cloud.
    #[arg(long, env = "TILR_CLOUDFLARED", default_value = "cloudflared")]
    cloudflared: PathBuf,

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

    /// Serve a SOCKS5 proxy on the same port, so a browser elsewhere reaches
    /// what listens on this machine: the WebSocket at /socks (through a
    /// tunnel too) and raw SOCKS5 with the access token as the password.
    /// Also `socks = true` in the configuration file.
    #[arg(long)]
    socks: bool,

    /// Do not offer the files of this machine to clients (no file browser,
    /// no editing). Also `[files] enabled = false` in the configuration
    /// file, which says which directories are offered.
    #[arg(long)]
    no_files: bool,

    /// Do not reload when the executable on disk changes. SIGUSR1 still does.
    #[arg(long)]
    no_auto_reload: bool,

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
    /// List the panes of a running server: id, pid of the pane process,
    /// size, viewers, directory and command.
    Ps {
        /// Where the server listens.
        #[arg(long, default_value = "ws://127.0.0.1:7890")]
        url: String,
    },
    /// Print the access token clients must present.
    Token,
    /// Link this machine to tilr Cloud, which lists it among your machines
    /// and gives it a tunnel.
    Cloud {
        #[command(subcommand)]
        command: CloudCommand,
    },
    /// Replace this executable with the latest release. A running server
    /// reloads itself and keeps its shells.
    Update {
        /// Where releases are fetched from.
        #[arg(long, env = "TILR_CDN", default_value = update::DEFAULT_CDN)]
        cdn: String,
    },
    /// Run in a pane: ask the tilr client watching it to open something.
    #[command(flatten)]
    Host(host::Command),
}

#[derive(Subcommand)]
enum CloudCommand {
    /// Link this machine to your account: shows a code to confirm in a
    /// browser.
    Login {
        /// Where tilr Cloud is.
        #[arg(long, env = "TILR_CLOUD", default_value = cloud::DEFAULT_URL)]
        url: String,
        /// What to call this machine there. The host name, if not given.
        #[arg(long)]
        name: Option<String>,
    },
    /// Unlink this machine and remove its tunnel.
    Logout,
    /// Say whether this machine is linked.
    Status,
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
    // Started as `tilr` through the link the server puts on the PATH of its
    // panes.
    if std::env::args_os()
        .next()
        .is_some_and(|argv0| host::is_shim(&argv0))
    {
        return Ok(host::shim(exe, args.collect()));
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Pane(args)) => pane::run(args)?,
        Some(Command::StateFormats) => print!("{}", reload::formats()),
        Some(Command::Ps { url }) => ps::run(&url)?,
        Some(Command::Token) => auth::print()?,
        Some(Command::Cloud { command }) => match command {
            CloudCommand::Login { url, name } => cloud::login(&url, name)?,
            CloudCommand::Logout => cloud::logout()?,
            CloudCommand::Status => cloud::status()?,
        },
        Some(Command::Update { cdn }) => update::run(&cdn, exe)?,
        Some(Command::Host(command)) => return Ok(host::run(command)),
        None => {
            let config = config::load(cli.config.as_deref())?;
            let allow_origins = config::allow_origins(cli.allow_origins, &config);
            tracing::info!(origins = ?allow_origins, "browser origins allowed");
            supervisor::run(supervisor::Options {
                listen: cli.listen,
                allow_origins,
                clipboard: (!cli.no_clipboard).then(|| clipboard::Limits {
                    max_bytes: cli.clipboard_max,
                    ttl: (cli.clipboard_ttl > 0).then(|| Duration::from_secs(cli.clipboard_ttl)),
                }),
                auth: !cli.no_auth,
                cloudflared: cli.cloudflared,
                auto_reload: !cli.no_auto_reload,
                client_timeout: (cli.client_timeout > 0)
                    .then(|| Duration::from_secs(cli.client_timeout)),
                socks: config::socks(cli.socks, &config),
                files: config::files(cli.no_files, &config)?,
            })?
        }
    }
    Ok(ExitCode::SUCCESS)
}
