//! Helpers of `sandbox-linux` and `sandbox-macos` for the bus tests, beside `bus.rs`:
//! a profile server with a tree of directories to grant, the output of a sandboxed
//! pane's program read from a FIFO as it comes, and the end of a pane process.
//!
//! Owner: `sandbox-linux` (`sandbox-macos` appends its part).

#![allow(dead_code, reason = "each test binary uses some of the helpers")]

use std::io::Read;
use std::path::{Path, PathBuf};

use tilr_bus::Bus;
use tilr_bus::pane::PaneInfo;
use tokio::sync::watch;

use super::bus_profiles::{ProfileServer, create, start_full, write_profile};
use super::bus_streams::within;

/// A profile server and a tree in its scratch directory: `w` to grant `read_write`,
/// `r` to grant `read_only`, `o` (another directory of the user) granted nothing.
pub struct Boxed {
    pub server: ProfileServer,
    pub root: PathBuf,
}

impl std::ops::Deref for Boxed {
    type Target = ProfileServer;

    fn deref(&self) -> &ProfileServer {
        &self.server
    }
}

impl Boxed {
    /// Start a server with `TILR_FAILPOINTS` set to `failpoints`, and make the tree.
    pub fn start(failpoints: &str) -> Self {
        Self::start_with_env(failpoints, &[])
    }

    /// [`Boxed::start`], with more environment `env` of the server.
    pub fn start_with_env(failpoints: &str, env: &[(&str, &str)]) -> Self {
        let server = start_full("", failpoints, env);
        let root = server.dir.join("box");
        for dir in ["w", "r", "o"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self { server, root }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn text(&self, name: &str) -> String {
        self.path(name).to_str().unwrap().to_owned()
    }

    /// The `[sandbox]` table granting `w` and `r`, then the lines `more` of it.
    pub fn grants(&self, more: &str) -> String {
        format!(
            "[sandbox]\nread_write = [{:?}]\nread_only = [{:?}]\n{more}\n",
            self.text("w"),
            self.text("r"),
        )
    }

    /// Write the profile `name`: `top` (keys and tables before `[sandbox]`), then
    /// [`Boxed::grants`] with `more`.
    pub fn profile(&self, name: &str, top: &str, more: &str) {
        write_profile(&self.server, name, &format!("{top}\n{}", self.grants(more)));
    }

    /// A FIFO in `w` that a sandboxed pane may write to.
    pub fn out(&self, name: &str) -> Out {
        Out::new(self.path(&format!("w/{name}")))
    }

    /// Run `script` with `sh -c` in a pane of the profile `profile` (the request's
    /// command), its output and errors to an [`Out`] in `w`; the pane and all it
    /// printed once the script ended.
    pub async fn run(&self, bus: &Bus, profile: &str, script: &str) -> (PaneInfo, String) {
        let mut out = self.out("run.out");
        let command = format!("( {script} ) > '{}' 2>&1", out.path.display());
        let pane = create(bus, profile, &["/bin/sh", "-c", &command], "")
            .await
            .expect("pane.create");
        let text = out.end().await;
        std::fs::remove_file(&out.path).unwrap();
        (pane, text)
    }
}

/// A FIFO a pane's program writes to, read by the test as the bytes come and to its
/// end, when every writer has closed it: waits are on those reads, not on a timer.
pub struct Out {
    pub path: PathBuf,
    /// What came so far, and whether the end has.
    seen: watch::Receiver<(Vec<u8>, bool)>,
}

impl Out {
    pub fn new(path: PathBuf) -> Self {
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo");
        assert!(made.success(), "mkfifo {}", path.display());
        let (tx, seen) = watch::channel((Vec::new(), false));
        let fifo = path.clone();
        // A thread of its own: the open waits for the writer, the reads for its bytes;
        // the test process ends it if no writer comes.
        std::thread::spawn(move || {
            let Ok(mut file) = std::fs::File::open(&fifo) else {
                return;
            };
            let mut buf = [0u8; 64 << 10];
            while let Ok(n @ 1..) = file.read(&mut buf) {
                tx.send_modify(|(seen, _)| seen.extend_from_slice(&buf[..n]));
            }
            tx.send_modify(|(_, ended)| *ended = true);
        });
        Self { path, seen }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.borrow().0).into_owned()
    }

    /// Wait until what came holds `needle`; all that came so far.
    pub async fn until(&mut self, needle: &str) -> String {
        let found = tokio::time::timeout(
            super::TIMEOUT,
            self.seen
                .wait_for(|(seen, _)| String::from_utf8_lossy(seen).contains(needle)),
        )
        .await
        .is_ok_and(|found| found.is_ok());
        assert!(
            found,
            "{needle:?} never came to {}; it holds:\n{}",
            self.path.display(),
            self.text()
        );
        self.text()
    }

    /// Wait for the end; all that came.
    pub async fn end(&mut self) -> String {
        let ended = tokio::time::timeout(super::TIMEOUT, self.seen.wait_for(|(_, ended)| *ended))
            .await
            .is_ok_and(|ended| ended.is_ok());
        assert!(
            ended,
            "{} did not end; it holds:\n{}",
            self.path.display(),
            self.text()
        );
        self.text()
    }
}

/// A process that is not a child of the test, to wait for its end (`pidfd_open`).
#[cfg(target_os = "linux")]
pub struct Exit(std::os::fd::OwnedFd);

#[cfg(target_os = "linux")]
impl Exit {
    /// Watch the process `pid`; it must run now.
    pub fn watch(pid: u32) -> Self {
        use std::os::fd::FromRawFd;
        // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        assert!(
            fd >= 0,
            "pidfd_open {pid}: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: a new descriptor that nothing else owns.
        Self(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) })
    }

    /// Wait until the process has ended (its pidfd becomes readable).
    pub async fn ended(self) {
        use std::os::fd::AsRawFd;
        let fd = self.0;
        let waited = tokio::task::spawn_blocking(move || {
            let mut poll = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let limit = super::TIMEOUT.as_millis() as libc::c_int;
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut poll, 1, limit) }
        });
        let ready = within(waited).await.unwrap();
        assert_eq!(ready, 1, "the process did not end in time");
    }
}

/// The private directory and the request file of the sandboxed pane `pane_id` of a
/// server whose runtime directory is `runtime`.
pub fn leftovers(runtime: &Path, pane_id: &str) -> [PathBuf; 2] {
    let dir = runtime.join("sandbox");
    [dir.join(pane_id), dir.join(format!("{pane_id}.request"))]
}

/// Wait until the screen of `watch` shows `text`; fails with the screen if it does not
/// within [`super::TIMEOUT`] (less than the wait for each frame).
pub async fn shows(watch: &mut super::bus_panes::Watching, text: &str) {
    let limit = super::TIMEOUT - std::time::Duration::from_secs(5);
    if tokio::time::timeout(limit, watch.wait_screen(text))
        .await
        .is_err()
    {
        panic!("never shown: {text:?}; screen:\n{}", watch.screen.text());
    }
}
