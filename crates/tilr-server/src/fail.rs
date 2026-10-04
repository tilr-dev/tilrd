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
//! - `sleep:MS` — the process stops there for a while.
//!
//! Compiled in with the `failpoints` feature only. A release has none of it.

use anyhow::Result;

/// A place where a test may make things go wrong.
#[cfg(not(feature = "failpoints"))]
#[inline(always)]
pub fn point(_name: &str) -> Result<()> {
    Ok(())
}

#[cfg(feature = "failpoints")]
pub fn point(name: &str) -> Result<()> {
    let points = std::env::var("TILR_FAILPOINTS").unwrap_or_default();
    let action = points
        .split(';')
        .filter_map(|point| point.split_once('='))
        .find_map(|(point, action)| (point == name).then_some(action));
    match action {
        None => Ok(()),
        Some("error") => anyhow::bail!("failpoint {name}"),
        Some("panic") => panic!("failpoint {name}"),
        Some("exit") => std::process::exit(1),
        Some("kill") => {
            // SAFETY: kill has no memory-safety preconditions.
            unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
            unreachable!("killed")
        }
        Some(action) => match action.strip_prefix("sleep:").map(str::parse) {
            Some(Ok(ms)) => {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(())
            }
            _ => panic!("failpoint {name}: unknown action {action}"),
        },
    }
}
