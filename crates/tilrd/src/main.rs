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
mod notify;
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
    /// The TCP address for WebSocket clients, which present the access
    /// token: the Tilr app, remote clients through an SSH or Cloud tunnel.
    /// Loopback unless told otherwise, e.g. 0.0.0.0:7890 or [::]:7890.
    #[arg(long, env = "TILR_LISTEN", default_value = "127.0.0.1:7890")]
    listen: SocketAddr,

    /// No TCP port: serve the Unix socket only.
    #[arg(long)]
    no_listen: bool,

    /// The Unix socket for local clients (`tilr`), which need no token.
    /// Default: $TILR_SOCKET, else tilrd.sock in $XDG_RUNTIME_DIR/tilr, else
    /// in tilr-<uid> in the temporary directory. With a TCP port and no
    /// --socket, a socket that cannot be taken (another tilrd serves it) is
    /// a warning, not an error.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Serve no Unix socket (not with --no-listen).
    #[arg(long, conflicts_with = "socket")]
    no_socket: bool,

    /// Internal: the descriptor of the socket's lock file, locked by the
    /// `tilr` that started this server. Such a server goes on without TCP
    /// when its port is taken (the Tilr app's server, another user's): the
    /// socket is what `tilr` needs.
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
    /// Replace this executable and the tilr next to it with the latest
    /// release. A running server reloads itself and keeps its shells. For
    /// users: `tilr update`, which runs this in the server.
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

    // Started by a reloading server or pane process to show that this
    // executable can resume from the state it would be left. Nothing is
    // logged: the answer goes to stdout.
    if let Some(state) = std::env::var_os(reload::CHECK_ENV) {
        return Ok(reload::answer_check(|| check_resume(Path::new(&state))));
    }

    // A supervisor exec'd by a reload starts with SIGUSR1, SIGHUP and
    // SIGTERM held: note what came meanwhile and let them in again before
    // any thread or child is started.
    reload::take_held_signals();

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
            let options = server_options(cli)?;
            tracing::info!(origins = ?options.allow_origins, "browser origins allowed");
            supervisor::run(options)?
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// What the command line and the configuration file make of a server.
fn server_options(cli: Cli) -> anyhow::Result<supervisor::Options> {
    if cli.no_listen && cli.no_socket {
        anyhow::bail!("nothing to listen on: --no-socket and --no-listen");
    }
    let listen = (!cli.no_listen).then_some(supervisor::ListenOptions {
        addr: cli.listen,
        required: cli.lock_fd.is_none(),
    });
    let local = (!cli.no_socket).then(|| supervisor::LocalOptions {
        // A socket asked for by name, or the only way in, must work.
        required: cli.socket.is_some() || cli.no_listen,
        path: cli.socket.unwrap_or_else(tilr_proto::local::socket_path),
        lock_fd: cli.lock_fd,
    });
    let config = config::load(cli.config.as_deref())?;
    let allow_origins = config::allow_origins(cli.allow_origins, &config);
    Ok(supervisor::Options {
        listen,
        local,
        allow_origins,
        clipboard: (!cli.no_clipboard).then(|| clipboard::Limits {
            max_bytes: cli.clipboard_max,
            ttl: (cli.clipboard_ttl > 0).then(|| Duration::from_secs(cli.clipboard_ttl)),
        }),
        auth: !cli.no_auth,
        cloudflared: cli.cloudflared,
        auto_reload: !cli.no_auto_reload,
        client_timeout: (cli.client_timeout > 0).then(|| Duration::from_secs(cli.client_timeout)),
        socks: config::socks(cli.socks, &config),
        files: config::files(cli.no_files, &config)?,
        agent: config::agent(cli.no_agent, &config),
        exec: config::exec(cli.no_exec, &config),
    })
}

/// The check child of a reload (see [`reload`]): take the command line and
/// the configuration as the successor will, and resume from the copy of the
/// state at `state` up to where the successor would take over.
fn check_resume(state: &Path) -> anyhow::Result<()> {
    let cli = Cli::try_parse().map_err(|err| {
        let message = err.to_string();
        let first = message.lines().next().unwrap_or_default();
        anyhow::anyhow!(
            "the command line: {}",
            first.strip_prefix("error: ").unwrap_or(first)
        )
    })?;
    match cli.command {
        None => supervisor::check(&server_options(cli)?, state),
        Some(Command::Pane(args)) => pane::check(&args, state),
        Some(_) => anyhow::bail!("not the command line of a server or a pane process"),
    }
}
