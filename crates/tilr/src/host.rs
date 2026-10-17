//! `tilr edit` / `tilr open` in a pane: ask the clients watching the pane to
//! open something. The same request, on the same socket (`$TILR_SOCK`), as
//! the `tilr` that tilrd puts first on the PATH of its panes (tilrd's
//! `host` module, which documents the protocol): a pane whose PATH was
//! reset finds this one, and it works the same.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The socket of the server the pane belongs to. Set for every pane.
const SOCK_ENV: &str = "TILR_SOCK";
/// The pane's id. Set for every pane.
const PANE_ENV: &str = "TILR_PANE";
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct Request<'a> {
    pane_id: &'a str,
    name: &'a str,
    kind: &'a str,
    path: &'a str,
    args: &'a [String],
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Ask for `target` to be opened as `kind` (none: a URL if it has a
/// scheme, else a file).
pub fn open(kind: Option<String>, target: &str, args: &[String]) -> ExitCode {
    let kind = kind.unwrap_or_else(|| if has_scheme(target) { "url" } else { "file" }.into());
    let path = if kind == "file" {
        match std::path::absolute(target) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(err) => {
                eprintln!("tilr: {target}: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        target.to_owned()
    };
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(pane_id), Some(sock)) = (var(PANE_ENV), var(SOCK_ENV)) else {
        eprintln!("tilr: not inside a tilr pane");
        return ExitCode::FAILURE;
    };
    let request = Request {
        pane_id: &pane_id,
        name: "open",
        kind: &kind,
        path: &path,
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
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    serde_json::from_str(&answer).with_context(|| format!("bad answer: {answer:?}"))
}

#[cfg(test)]
mod tests {
    use super::has_scheme;

    #[test]
    fn urls_are_told_from_files() {
        assert!(has_scheme("https://example.com/a"));
        assert!(!has_scheme("notes.md"));
        assert!(!has_scheme("dir/with://odd"));
    }
}
