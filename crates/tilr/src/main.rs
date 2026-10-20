//! `tilr`: the terminal client of tilrd, tmux-style. With no server
//! running, it starts the local one (detached) and connects over its Unix
//! socket; see `tilr --help`.

mod attach;
mod conn;
mod daemon;
mod host;
mod service;
mod target;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tilr_proto::PaneInfo;

use conn::{Conn, ConnectError};
use target::Target;

#[derive(Parser)]
#[command(
    name = "tilr",
    version = concat!(env!("TILR_VERSION"), " (", env!("TILR_PLATFORM"), ")"),
    about = "Terminal client of tilrd: attach to its panes from a terminal, tmux-style",
    long_about = "Terminal client of tilrd: attach to its panes from a terminal, tmux-style.\n\n\
        With no command: attach to the pane used last, or make one. The local tilrd is \
        reached over its Unix socket and started if it is not running.\n\n\
        Attached: Ctrl+] d detaches, Ctrl+] Ctrl+] sends Ctrl+]."
)]
struct Cli {
    /// The local tilrd's Unix socket. Default: tilrd.sock in
    /// $XDG_RUNTIME_DIR/tilr, else in tilr-<uid> in the temporary directory.
    #[arg(long, env = "TILR_SOCKET", global = true, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// A tilrd's TCP port instead, e.g. ws://10.0.0.5:7890. It is not
    /// started from here.
    #[arg(long, env = "TILR_CONNECT", global = true, value_name = "URL")]
    connect: Option<String>,

    /// The access token of the server at --connect (`tilr token` there).
    #[arg(long, env = "TILR_TOKEN", global = true, hide_env_values = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Make a pane and attach to it.
    New {
        /// Only make it and print its id.
        #[arg(short, long)]
        detach: bool,
        /// Directory to start in. Default: the current one.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// What to run instead of the login shell.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Attach to a pane: its id or the start of it. Default: the pane used
    /// last.
    #[command(visible_alias = "a")]
    Attach { pane: Option<String> },
    /// List the panes, oldest first; `*` marks the one used last.
    #[command(visible_alias = "list")]
    Ls {
        /// Print the full ids only, one per line.
        #[arg(short, long)]
        quiet: bool,
    },
    /// Close a pane (its shell is hung up). Default: the one this runs in,
    /// else the one used last.
    Kill { pane: Option<String> },
    /// Stop the local tilrd: every pane's shell ends, its socket goes.
    KillServer,
    /// The panes with the pids of their processes, as the server lists them.
    Ps,
    /// This client's version and the server's.
    Version,
    /// Print the access token clients on the server's TCP port present.
    Token,
    /// Link this machine to tilr Cloud, which lists it among your machines
    /// and gives the server a tunnel (to its TCP port).
    Cloud {
        #[command(subcommand)]
        command: CloudCommand,
    },
    /// Replace tilrd and tilr with their latest release; tilrd reloads and
    /// keeps its shells.
    Update {
        /// Where releases are fetched from.
        #[arg(long, env = "TILR_CDN")]
        cdn: Option<String>,
    },
    /// Start tilrd at boot, as a service of yours (systemd user unit, else
    /// a crontab @reboot line; launchd on macOS), or stop doing so. Nothing
    /// starts it at boot otherwise.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// In a pane: open a file in the tilr client watching it.
    Edit {
        /// Relative to the current directory.
        path: PathBuf,
    },
    /// In a pane: ask the tilr client watching it to open something.
    Open {
        /// What it is: "file", "url", or a kind a client knows. Default: url
        /// for something with a scheme (https://…), file otherwise.
        #[arg(long)]
        kind: Option<String>,
        /// A file (relative to the current directory) or a URL.
        target: String,
        /// Passed on to the client.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Install the service and start it (unless a tilrd runs already).
    Install,
    /// Remove what install added; a running tilrd goes on.
    Uninstall,
}

#[derive(Subcommand)]
enum CloudCommand {
    /// Link this machine to your account: shows a code to confirm in a
    /// browser.
    Login {
        /// Where tilr Cloud is.
        // tilrd's `cloud::DEFAULT_URL`; a test keeps the two helps alike.
        #[arg(long, env = "TILR_CLOUD", default_value = "https://wss.tilr.dev")]
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Requests from inside a pane need no server connection of their own.
    match &cli.command {
        Some(Command::Edit { path }) => {
            return host::open(Some("file".into()), &path.to_string_lossy(), &[]);
        }
        Some(Command::Open { kind, target, args }) => {
            return host::open(kind.clone(), target, args);
        }
        Some(Command::Service { command }) => {
            let socket = cli
                .socket
                .clone()
                .unwrap_or_else(tilr_proto::local::socket_path);
            let done = match command {
                ServiceCommand::Install => service::install(&socket),
                ServiceCommand::Uninstall => service::uninstall(),
            };
            return done.unwrap_or_else(|err| {
                eprintln!("tilr: {err:#}");
                ExitCode::FAILURE
            });
        }
        _ => {}
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("tilr: {err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("tilr: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let target = target::resolve(
        target::Given {
            socket: cli.socket,
            connect: cli.connect,
            token: cli.token,
        },
        target::load_config()?,
    )?;
    // One id for this client for as long as it runs, the same on every
    // connection it makes: a reconnect supersedes what the last one left.
    let client_id = uuid::Uuid::new_v4().to_string();
    let command = cli.command.unwrap_or(Command::Attach { pane: None });

    if let Command::KillServer = command {
        return kill_server(&target, &client_id).await;
    }
    if let Command::Version = command {
        println!("tilr {} ({})", env!("TILR_VERSION"), env!("TILR_PLATFORM"));
        match Conn::connect(&target, &client_id).await {
            Ok(conn) => println!("tilrd {} at {target}", conn.server_version),
            Err(ConnectError::NoServer(_)) => println!("tilrd is not running at {target}"),
            Err(err) => return Err(err.into()),
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(args) = server_command(&command) {
        if let Target::Remote { .. } = target {
            bail!("server commands run in the local tilrd; on another machine, run tilr there");
        }
        let mut conn = connect(&target, &client_id, true).await?;
        let update = matches!(command, Command::Update { .. });
        let exit = conn.server_command(&args).await?;
        if update && exit.reason == "reload" {
            // The update replaced the executable and the server reloaded
            // into it, ending the command with it.
            return after_update(&target, &client_id).await;
        }
        if !exit.reason.is_empty() {
            bail!("the server ended the command: {}", exit.reason);
        }
        if update && exit.code == 0 {
            return after_update(&target, &client_id).await;
        }
        return Ok(match (exit.code, exit.signal) {
            (0, 0) => ExitCode::SUCCESS,
            (code, 0) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            _ => ExitCode::FAILURE,
        });
    }
    // A pane already there, or the server stopped: nothing to start.
    let start = matches!(
        command,
        Command::New { .. } | Command::Attach { pane: None }
    );
    let mut conn = connect(&target, &client_id, start).await?;
    match command {
        Command::New {
            detach,
            cwd,
            command,
        } => {
            let (cols, rows) = size();
            let cwd = match cwd {
                Some(cwd) => std::path::absolute(cwd)?,
                None => std::env::current_dir().unwrap_or_default(),
            };
            let pane = conn
                .create_pane(cols, rows, command, cwd.to_string_lossy().into_owned())
                .await
                .context("make a pane")?;
            if detach {
                println!("{}", pane.pane_id);
                return Ok(ExitCode::SUCCESS);
            }
            attach(conn, &target, &client_id, &pane.pane_id).await
        }
        Command::Attach { pane } => {
            let panes = conn.list_panes().await?;
            let own = own_pane();
            let pane_id = match pane {
                Some(named) => find(&panes, &named)?.pane_id.clone(),
                None => match last_used(&panes, own.as_deref()) {
                    Some(pane) => pane.pane_id.clone(),
                    None => {
                        let (cols, rows) = size();
                        let cwd = std::env::current_dir().unwrap_or_default();
                        conn.create_pane(cols, rows, Vec::new(), cwd.to_string_lossy().into_owned())
                            .await
                            .context("make a pane")?
                            .pane_id
                    }
                },
            };
            if own.as_deref() == Some(pane_id.as_str()) {
                bail!("{pane_id} is the pane this runs in: it cannot show itself");
            }
            attach(conn, &target, &client_id, &pane_id).await
        }
        Command::Ls { quiet } => {
            let panes = conn.list_panes().await?;
            if quiet {
                for pane in &panes {
                    println!("{}", pane.pane_id);
                }
            } else {
                print!("{}", table(&panes));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Kill { pane } => {
            let panes = conn.list_panes().await?;
            let pane = match (pane, own_pane()) {
                (Some(named), _) => find(&panes, &named)?,
                (None, Some(own)) => find(&panes, &own)?,
                (None, None) => last_used(&panes, None).context("no panes")?,
            };
            let pane_id = pane.pane_id.clone();
            conn.close_pane(&pane_id).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Ps => {
            print!("{}", ps_table(&conn.list_panes().await?));
            Ok(ExitCode::SUCCESS)
        }
        Command::KillServer
        | Command::Version
        | Command::Token
        | Command::Cloud { .. }
        | Command::Update { .. }
        | Command::Edit { .. }
        | Command::Open { .. }
        | Command::Service { .. } => unreachable!(),
    }
}

/// The arguments of tilrd for a command run in the server.
fn server_command(command: &Command) -> Option<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    match command {
        Command::Token => args.push("token".into()),
        Command::Cloud { command } => {
            args.push("cloud".into());
            match command {
                CloudCommand::Login { url, name } => {
                    args.push("login".into());
                    args.extend(["--url".into(), url.clone()]);
                    if let Some(name) = name {
                        args.extend(["--name".into(), name.clone()]);
                    }
                }
                CloudCommand::Logout => args.push("logout".into()),
                CloudCommand::Status => args.push("status".into()),
            }
        }
        Command::Update { cdn } => {
            args.push("update".into());
            if let Some(cdn) = cdn {
                args.extend(["--cdn".into(), cdn.clone()]);
            }
        }
        _ => return None,
    }
    Some(args)
}

/// Once an update is in: the server, reloaded, says its version.
async fn after_update(target: &Target, client_id: &str) -> Result<ExitCode> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match Conn::connect(target, client_id).await {
            Ok(conn) => {
                println!("tilrd {} is running", conn.server_version);
                return Ok(ExitCode::SUCCESS);
            }
            Err(err) if tokio::time::Instant::now() >= deadline => {
                bail!("the server did not come back after the update: {err}")
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
}

/// Connect; for the local server, start it first if it is not running and
/// `start` says to.
async fn connect(target: &Target, client_id: &str, start: bool) -> Result<Conn> {
    if let (Target::Local(socket), true) = (target, start) {
        let socket = socket.clone();
        tokio::task::spawn_blocking(move || daemon::ensure_running(&socket)).await??;
    }
    match Conn::connect(target, client_id).await {
        Ok(conn) => Ok(conn),
        Err(ConnectError::NoServer(err)) => match target {
            Target::Local(socket) => {
                bail!("no tilrd runs at {} ({err})", socket.display())
            }
            Target::Remote { url, .. } => bail!("no tilrd answers at {url} ({err})"),
        },
        Err(ConnectError::Other(err)) => match target {
            Target::Local(socket) if daemon::stopped(socket) => bail!(
                "something that is not tilrd listens on {} (nothing was removed): {err:#}",
                socket.display()
            ),
            _ => Err(err),
        },
        Err(ConnectError::Untrusted(err)) => Err(err),
    }
}

async fn attach(conn: Conn, target: &Target, client_id: &str, pane_id: &str) -> Result<ExitCode> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        bail!("attaching needs a terminal (`tilr ls`, `tilr new -d` do not)");
    }
    let short = &pane_id[..pane_id.len().min(8)];
    let exit = attach::run(conn, target, client_id, pane_id).await?;
    match exit {
        attach::Exit::Detached => println!("[detached from pane {short}]"),
        attach::Exit::PaneExited(code) => println!("[pane {short} exited with code {code}]"),
        attach::Exit::PaneGone => println!("[pane {short} is gone]"),
        attach::Exit::ServerGone => println!("[tilrd stopped]"),
        attach::Exit::Superseded => println!("[superseded by another connection of this client]"),
    }
    Ok(ExitCode::SUCCESS)
}

async fn kill_server(target: &Target, client_id: &str) -> Result<ExitCode> {
    let Target::Local(socket) = target else {
        bail!("kill-server stops the local tilrd; a server at --connect is stopped where it runs");
    };
    let mut conn = match Conn::connect(target, client_id).await {
        Ok(conn) => conn,
        Err(ConnectError::NoServer(_)) => bail!("no tilrd runs at {}", socket.display()),
        Err(err) => return Err(err.into()),
    };
    let server = conn
        .server_pid()
        .context("the system does not say which process the server is")?;
    let panes: Vec<i32> = conn
        .list_panes()
        .await?
        .iter()
        .map(|pane| pane.pid as i32)
        .collect();
    drop(conn);
    let socket = socket.clone();
    tokio::task::spawn_blocking(move || daemon::stop(&socket, server, &panes)).await??;
    Ok(ExitCode::SUCCESS)
}

/// The pane this runs in, if any.
fn own_pane() -> Option<String> {
    std::env::var("TILR_PANE").ok().filter(|id| !id.is_empty())
}

/// The pane used last, other than `except`.
fn last_used<'a>(panes: &'a [PaneInfo], except: Option<&str>) -> Option<&'a PaneInfo> {
    panes
        .iter()
        .filter(|pane| Some(pane.pane_id.as_str()) != except)
        .max_by_key(|pane| (pane.used_at_ms.max(pane.created_at_ms), pane.created_at_ms))
}

/// A pane by its id or the start of it.
fn find<'a>(panes: &'a [PaneInfo], named: &str) -> Result<&'a PaneInfo> {
    if let Some(pane) = panes.iter().find(|pane| pane.pane_id == named) {
        return Ok(pane);
    }
    let matching: Vec<_> = panes
        .iter()
        .filter(|pane| !named.is_empty() && pane.pane_id.starts_with(named))
        .collect();
    match matching.as_slice() {
        [pane] => Ok(pane),
        [] => bail!("no pane {named} (tilr ls lists them)"),
        _ => bail!(
            "{named} names {} panes: give more of the id",
            matching.len()
        ),
    }
}

/// This terminal's size, or 80x24 when there is none.
fn size() -> (u16, u16) {
    crossterm::terminal::size()
        .ok()
        .filter(|&(cols, rows)| cols > 0 && rows > 0)
        .unwrap_or((80, 24))
}

/// `tilr ps`: one line per pane with its full id and the pid of its
/// process (the shell is its child).
fn ps_table(panes: &[PaneInfo]) -> String {
    let mut out = format!(
        "{:<36}  {:>7}  {:>7}  {:>7}  {}\n",
        "PANE", "PID", "SIZE", "VIEWERS", "CWD  COMMAND"
    );
    for pane in panes {
        let command = match pane.command.as_slice() {
            [] => "(shell)".to_owned(),
            command => command.join(" "),
        };
        out.push_str(&format!(
            "{:<36}  {:>7}  {:>7}  {:>7}  {}  {}\n",
            pane.pane_id,
            pane.pid,
            format!("{}x{}", pane.cols, pane.rows),
            pane.viewers,
            pane.cwd,
            command
        ));
    }
    out
}

/// One line per pane: short id, size, viewers, directory, command.
fn table(panes: &[PaneInfo]) -> String {
    let last = last_used(panes, None).map(|pane| pane.pane_id.as_str());
    let mut out = format!(
        "  {:<8}  {:>7}  {:>7}  {}\n",
        "PANE", "SIZE", "VIEWERS", "CWD  COMMAND"
    );
    for pane in panes {
        let mark = if Some(pane.pane_id.as_str()) == last {
            '*'
        } else {
            ' '
        };
        let command = match pane.command.as_slice() {
            [] => "(shell)".to_owned(),
            command => command.join(" "),
        };
        out.push_str(&format!(
            "{mark} {:<8}  {:>7}  {:>7}  {}  {}\n",
            &pane.pane_id[..pane.pane_id.len().min(8)],
            format!("{}x{}", pane.cols, pane.rows),
            pane.viewers,
            pane.cwd,
            command
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, created: u64, used: u64) -> PaneInfo {
        PaneInfo {
            pane_id: id.into(),
            created_at_ms: created,
            used_at_ms: used,
            ..Default::default()
        }
    }

    #[test]
    fn the_pane_used_last_is_attached_to() {
        let panes = [pane("aaaa", 1, 50), pane("bbbb", 2, 0), pane("cccc", 3, 10)];
        assert_eq!(last_used(&panes, None).unwrap().pane_id, "aaaa");
        assert_eq!(last_used(&panes, Some("aaaa")).unwrap().pane_id, "cccc");
        assert!(last_used(&[], None).is_none());
    }

    #[test]
    fn a_pane_is_named_by_the_start_of_its_id() {
        let panes = [pane("ab12", 1, 0), pane("ab34", 2, 0), pane("cd56", 3, 0)];
        assert_eq!(find(&panes, "cd").unwrap().pane_id, "cd56");
        assert_eq!(find(&panes, "ab34").unwrap().pane_id, "ab34");
        assert!(
            find(&panes, "ab")
                .unwrap_err()
                .to_string()
                .contains("2 panes")
        );
        assert!(find(&panes, "zz").is_err());
        assert!(find(&panes, "").is_err());
    }
}
