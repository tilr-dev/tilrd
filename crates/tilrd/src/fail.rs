//! Fault injection, for the tests of what happens when an update or a reload
//! goes wrong halfway.
//!
//! The code names the places where something can go wrong; a test says what
//! happens there with `TILR_FAILPOINTS`, a `;`-separated list of
//! `name=action`:
//!
//! - `error` — the step fails, as if the system had refused it;
//! - `panic`, `exit` — the process ends there on its own;
//! - `kill` — the process is killed there, as by `kill -9` or a power cut;
//! - `sleep:MS` — the process stops there for a while, logging
//!   `failpoint NAME: sleeping MS ms` first.
//! - `pause` — connect to the Unix stream socket `TILR_FAILPOINTS_SOCKET`,
//!   write `NAME\n`, then wait for exactly the byte `r` before returning.
//!   Each hit opens its own connection: a release never carries to another
//!   hit. Missing configuration, I/O errors, EOF and other bytes fail the
//!   point; disconnecting the controller does not silently release it.
//!
//! Bind the control socket before triggering a point. Tests use
//! `common::PauseController::bind(&server.dir)`, `wait("NAME").await`, then
//! `PausedPoint::release().await`. The harness sets the socket environment.
//! Keep the returned point alive while checking the state in the pause.
//! Release `supervisor-resume` and `pane-resume*` before the successor check's
//! `CHECK_TIMEOUT` (10 seconds), which terminates that check even while paused.
//!
//! `point` blocks its calling thread, with no timeout. In a Tokio task that
//! must leave the runtime responsive, use an owned name on a blocking thread:
//! `tokio::task::spawn_blocking(move || fail::point(&name)).await??`.
//! Do not hold a lock needed by the test or other tasks across the pause.
//! Aborting that task does not cancel its blocking thread: the controller
//! must release or close the connection (or terminate the test process).
//!
//! A test that only learns the name of a point once the server runs (the id of
//! a pane, say) puts more of the same into the file `TILR_FAILPOINTS_FILE`
//! names; it is read every time a point is passed.
//!
//! Compiled in with the `failpoints` feature only. A release has none of it.

use anyhow::Result;

/// Hold a test stage without blocking a Tokio runtime worker. A controller
/// failure must fail the operation rather than silently advance its ordering.
#[cfg(feature = "failpoints")]
pub async fn point_async(name: String) {
    tokio::task::spawn_blocking(move || point(&name))
        .await
        .expect("failpoint task panicked")
        .expect("failpoint failed");
}

/// A place where a test may make things go wrong.
#[cfg(not(feature = "failpoints"))]
#[inline(always)]
pub fn point(_name: &str) -> Result<()> {
    Ok(())
}

#[cfg(feature = "failpoints")]
pub fn point(name: &str) -> Result<()> {
    let mut points = std::env::var("TILR_FAILPOINTS").unwrap_or_default();
    if let Some(file) = std::env::var_os("TILR_FAILPOINTS_FILE") {
        points.push(';');
        points.push_str(std::fs::read_to_string(file).unwrap_or_default().trim());
    }
    let action = points
        .split(';')
        .filter_map(|point| point.split_once('='))
        .find_map(|(point, action)| (point == name).then_some(action));
    match action {
        None => Ok(()),
        Some("error") => anyhow::bail!("failpoint {name}"),
        Some("pause") => pause(name),
        Some("panic") => panic!("failpoint {name}"),
        Some("exit") => std::process::exit(1),
        Some("kill") => {
            // SAFETY: kill has no memory-safety preconditions.
            unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
            unreachable!("killed")
        }
        Some(action) => match action.strip_prefix("sleep:").map(str::parse) {
            Some(Ok(ms)) => {
                // A test waits for this line to act while the process sleeps.
                tracing::info!("failpoint {name}: sleeping {ms} ms");
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(())
            }
            _ => panic!("failpoint {name}: unknown action {action}"),
        },
    }
}

#[cfg(feature = "failpoints")]
fn pause(name: &str) -> Result<()> {
    use anyhow::Context;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let wait = || -> Result<()> {
        let path = std::env::var_os("TILR_FAILPOINTS_SOCKET")
            .context("TILR_FAILPOINTS_SOCKET is not set")?;
        let mut socket = UnixStream::connect(path).context("connect to pause controller")?;
        writeln!(socket, "{name}").context("announce paused point")?;
        let mut release = [0];
        socket
            .read_exact(&mut release)
            .context("wait for release")?;
        anyhow::ensure!(release == *b"r", "expected pause release byte r");
        Ok(())
    };
    wait().with_context(|| format!("failpoint {name}: pause"))
}
