mod agent;
mod auth;
mod clipboard;
mod cloud;
mod config;
mod exec;
mod fail;
mod files;
mod host;
mod identity;
mod link;
mod local;
mod metrics;
mod outbox;
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
    name = "tilrd",
    version = concat!(env!("TILR_VERSION"), " (", env!("TILR_PLATFORM"), ")"),
    about = "Terminal server: ptys behind a WebSocket",
    long_about = "Terminal server: ptys behind a WebSocket. A service: started by `tilr`, \
        a service manager or the Tilr app, not run by hand. Users type `tilr …` \
        (`tilr token`, `tilr cloud login`, `tilr update`, `tilr ps`)."
)]
struct Cli {
    /// Also listen for WebSocket clients on this TCP address, e.g.
    /// 127.0.0.1:7890 (the Tilr app, remote clients; they present the access
    /// token). Without it the server serves its Unix socket only.
    #[arg(long)]
    listen: Option<SocketAddr>,

    /// The Unix socket for local clients (`tilr`), which need no token.
    /// Default: $TILR_SOCKET, else tilrd.sock in $XDG_RUNTIME_DIR/tilr, else
    /// in tilr-<uid> in the temporary directory. With --listen and no
    /// --socket, a socket that cannot be taken (another tilrd serves it) is
    /// a warning, not an error.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Serve no Unix socket (needs --listen).
    #[arg(long, conflicts_with = "socket")]
    no_socket: bool,

    /// Internal: the descriptor of the socket's lock file, locked by the
    /// `tilr` that started this server.
    #[arg(long, hide = true)]
    lock_fd: Option<i32>,

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

    /// Do not forward the SSH agent of the clients to the panes (no
    /// SSH_AUTH_SOCK in them). Also `agent = false` in the configuration
    /// file.
    #[arg(long)]
    no_agent: bool,

    /// Do not let clients run programs on this machine outside a pane
    /// (Exec). Also `exec = false` in the configuration file.
    #[arg(long)]
    no_exec: bool,

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
    /// size, viewers, directory and command. For users: `tilr ps`; kept
    /// for the Tilr app, which asks with --url and --server-version.
    #[command(hide = true)]
    Ps {
        /// Where the server listens, e.g. ws://127.0.0.1:7890. Default: its
        /// Unix socket (see --socket).
        #[arg(long)]
        url: Option<String>,
        /// Internal health check: print the connected server's version.
        #[arg(long, hide = true)]
        server_version: bool,
    },
    /// Print the access token clients must present. For users: `tilr
    /// token`, which runs this in the server; kept for the Tilr app.
    #[command(hide = true)]
    Token,
    /// Link this machine to tilr Cloud, which lists it among your machines
    /// and gives it a tunnel. For users: `tilr cloud`, which runs this in the
    /// server.
    #[command(hide = true)]
    Cloud {
        #[command(subcommand)]
        command: CloudCommand,
    },
    /// Replace this executable with the latest release. A running server
    /// reloads itself and keeps its shells. For users: `tilr update`, which
    /// runs this in the server.
    #[command(hide = true)]
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

    // Write errors are dropped: stderr may be a terminal that hung up (EIO),
    // and reporting that to stderr again would panic.
    // Colors for a terminal only: the log of a server `tilr` started is a file.
    let colors = std::io::IsTerminal::is_terminal(&std::io::stderr())
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    tracing_subscriber::fmt()
        .with_ansi(colors)
        .with_writer(std::io::stderr)
        .log_internal_errors(false)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Pane(args)) => pane::run(args)?,
        Some(Command::StateFormats) => print!("{}", reload::formats()),
        Some(Command::Ps {
            url,
            server_version,
        }) => ps::run(url.as_deref(), server_version)?,
        Some(Command::Token) => auth::print()?,
        Some(Command::Cloud { command }) => match command {
            CloudCommand::Login { url, name } => cloud::login(&url, name)?,
            CloudCommand::Logout => cloud::logout()?,
            CloudCommand::Status => cloud::status()?,
        },
        Some(Command::Update { cdn }) => update::run(&cdn, exe)?,
        Some(Command::Host(command)) => return Ok(host::run(command)),
        None => {
            if cli.listen.is_none() && cli.no_socket {
                anyhow::bail!("nothing to listen on: --no-socket needs --listen");
            }
            let local = (!cli.no_socket).then(|| supervisor::LocalOptions {
                // A socket asked for by name, or the only way in, must work.
                required: cli.socket.is_some() || cli.listen.is_none(),
                path: cli.socket.unwrap_or_else(tilr_proto::local::socket_path),
                lock_fd: cli.lock_fd,
            });
            let config = config::load(cli.config.as_deref())?;
            let allow_origins = config::allow_origins(cli.allow_origins, &config);
            tracing::info!(origins = ?allow_origins, "browser origins allowed");
            supervisor::run(supervisor::Options {
                listen: cli.listen,
                local,
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
                agent: config::agent(cli.no_agent, &config),
                exec: config::exec(cli.no_exec, &config),
            })?
        }
    }
    Ok(ExitCode::SUCCESS)
}
