//! Host events: a program in a pane asks the client to do something, e.g.
//! `tilr edit notes.md` to open a file in the client's editor.
//!
//! The server listens on a Unix socket of its own ([`socket_path`]) and puts
//! its path in the environment of every pane ([`SOCK_ENV`], next to
//! `TILR_PANE` and [`SERVER_ENV`]). A program writes one JSON object per line:
//!
//! ```json
//! {"pane_id": "6f1c…", "name": "open", "kind": "file", "path": "/home/me/notes.md", "args": []}
//! ```
//!
//! and the server passes it on as `HostEvent` to every client subscribed to
//! that pane that announced "host-events" in its `Hello`, then answers `{"ok": true, "clients": 2}`, or `{"ok": false,
//! "error": "no tilr client attached"}` when nobody would hear it.
//!
//! The program is this executable under the name `tilr` (put on the PATH of
//! the panes, see [`shim`]) or `tilrd edit` / `tilrd open`.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use crate::clipboard;

/// The socket of the server the pane belongs to. Set for every pane.
pub const SOCK_ENV: &str = "TILR_SOCK";
/// The id of the server the pane belongs to. Set for every pane.
pub const SERVER_ENV: &str = "TILR_SERVER";
/// The name this executable answers to as the command for programs in panes.
pub const SHIM: &str = "tilr";
/// Longest path a Unix socket can be bound at (macOS allows 104 bytes with
/// the terminating zero, Linux 108).
const MAX_SOCKET_PATH: usize = 103;
/// Most a program may send on one connection.
const MAX_INPUT: u64 = 1 << 20;
/// How long `tilr edit` waits for the server.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// What a program in a pane asks for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub pane_id: String,
    #[serde(default = "open")]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn open() -> String {
    "open".into()
}

/// The server's answer.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clients: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Hands a request to the supervisor, which answers with the number of
/// clients it went to, or why it went to none.
pub type Deliver =
    Box<dyn Fn(Request, oneshot::Sender<Result<usize, String>>) -> bool + Send + Sync>;

/// Where the server with this id listens: the runtime directory
/// (`$XDG_RUNTIME_DIR/tilr`), on macOS `~/Library/Application Support/tilr`.
/// A path too long for a socket falls back to `/tmp/tilr-<uid>`.
pub fn socket_path(server_id: &str) -> PathBuf {
    runtime_socket(Path::new(&format!("{server_id}.sock")))
}

/// Where a socket of the server's named `name` (relative, possibly with a
/// directory) goes: as [`socket_path`]'s.
pub fn runtime_socket(name: &Path) -> PathBuf {
    let explicit = std::env::var_os("TILR_RUNTIME_DIR").is_some_and(|dir| !dir.is_empty())
        || std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| !dir.is_empty());
    let dir = match std::env::home_dir() {
        Some(home) if cfg!(target_os = "macos") && !explicit => {
            home.join("Library/Application Support/tilr")
        }
        _ => clipboard::runtime_dir(),
    };
    let path = dir.join(name);
    if path.as_os_str().len() <= MAX_SOCKET_PATH {
        return path;
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    Path::new("/tmp").join(format!("tilr-{uid}")).join(name)
}

/// Listen at `path`, replacing what a server that did not get to clean up
/// left there.
pub fn bind(path: &Path) -> Result<UnixListener> {
    let dir = path.parent().context("socket path has no directory")?;
    clipboard::private_dir(dir)?;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
        }
        Ok(_) => anyhow::bail!("{} is in the way: not a socket", path.display()),
        Err(_) => {}
    }
    UnixListener::bind(path).with_context(|| format!("listen on {}", path.display()))
}

/// Remove the socket file, when the server stops for good.
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Accept programs and pass on what they ask.
pub async fn serve(listener: UnixListener, deliver: Deliver) {
    let deliver = std::sync::Arc::new(deliver);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                tracing::warn!("host socket: accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let deliver = std::sync::Arc::clone(&deliver);
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut lines =
                tokio::io::BufReader::new(tokio::io::AsyncReadExt::take(read, MAX_INPUT)).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let reply = match serde_json::from_str::<Request>(&line) {
                    Ok(request) => {
                        let (tx, rx) = oneshot::channel();
                        if !deliver(request, tx) {
                            break;
                        }
                        match rx.await {
                            Ok(Ok(clients)) => Reply {
                                ok: true,
                                clients: Some(clients),
                                error: None,
                            },
                            Ok(Err(error)) => Reply {
                                ok: false,
                                clients: None,
                                error: Some(error),
                            },
                            Err(_) => break,
                        }
                    }
                    Err(err) => Reply {
                        ok: false,
                        clients: None,
                        error: Some(format!("bad request: {err}")),
                    },
                };
                let mut text = serde_json::to_string(&reply).expect("reply to JSON");
                text.push('\n');
                if write.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// The command: `tilr edit`, `tilr open`
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum Command {
    /// Open a file in the tilr client watching this pane: `tilr open --kind
    /// file <path>`.
    Edit {
        /// Relative to the current directory.
        path: PathBuf,
    },
    /// Ask the tilr client watching this pane to open something.
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

/// `tilr` in a pane: the host commands, as the name `tilr` takes them.
#[derive(Parser)]
#[command(
    name = "tilr",
    about = "Ask the tilr client watching this pane to open something"
)]
struct ShimCli {
    #[command(subcommand)]
    command: Command,
}

/// Whether this process was started as `tilr`.
pub fn is_shim(argv0: &std::ffi::OsStr) -> bool {
    Path::new(argv0)
        .file_name()
        .is_some_and(|name| name == SHIM)
}

/// Run as `tilr`: `edit` and `open` are answered here; anything else goes to
/// the next `tilr` on PATH (the Tilr app's, say).
pub fn shim(exe: &Path, args: Vec<OsString>) -> ExitCode {
    let ours = args
        .first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| matches!(arg, "edit" | "open" | "help" | "--help" | "-h"));
    if !ours {
        return clipboard::exec_next(SHIM, exe, &args, "no other tilr is installed");
    }
    let cli = ShimCli::parse_from(std::iter::once(OsString::from(SHIM)).chain(args));
    run(cli.command)
}

/// Send the request to the server of the pane this runs in.
pub fn run(command: Command) -> ExitCode {
    let request = match command {
        Command::Edit { path } => (
            String::from("file"),
            path.to_string_lossy().into_owned(),
            Vec::new(),
        ),
        Command::Open { kind, target, args } => {
            let kind =
                kind.unwrap_or_else(|| if has_scheme(&target) { "url" } else { "file" }.into());
            (kind, target, args)
        }
    };
    let (kind, target, args) = request;
    let path = if kind == "file" {
        match std::path::absolute(&target) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(err) => {
                eprintln!("tilr: {target}: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        target
    };
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(pane_id), Some(sock)) = (var(crate::pty::PANE_ENV), var(SOCK_ENV)) else {
        eprintln!("tilr: not inside a tilr pane");
        return ExitCode::FAILURE;
    };
    let request = Request {
        pane_id,
        name: "open".into(),
        kind,
        path,
        args,
    };
    match ask(Path::new(&sock), &request) {
        Ok(Reply { ok: true, .. }) => ExitCode::SUCCESS,
        Ok(Reply { error, .. }) => {
            eprintln!("tilr: {}", error.unwrap_or_else(|| "refused".into()));
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("tilr: cannot reach the tilr server: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn has_scheme(target: &str) -> bool {
    target.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

fn ask(sock: &Path, request: &Request) -> Result<Reply> {
    let mut stream =
        UnixStream::connect(sock).with_context(|| format!("connect to {}", sock.display()))?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    serde_json::from_str(&answer).with_context(|| format!("bad answer: {answer:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_needs_only_the_pane() {
        let request: Request = serde_json::from_str(r#"{"pane_id":"x"}"#).unwrap();
        assert_eq!(request.name, "open");
        assert_eq!(request.kind, "");
        assert!(request.args.is_empty());
    }

    #[test]
    fn urls_are_told_from_files() {
        assert!(has_scheme("https://example.com/a"));
        assert!(has_scheme("git+ssh://host/repo"));
        assert!(!has_scheme("notes.md"));
        assert!(!has_scheme("dir/with://odd"));
    }

    #[test]
    fn replies_carry_what_applies() {
        let ok = Reply {
            ok: true,
            clients: Some(2),
            error: None,
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"ok":true,"clients":2}"#
        );
    }
}
