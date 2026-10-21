//! Use cases of `tilr` with a real tilrd: starting the server tmux-style,
//! one server for many clients, kill-server, what is found at the socket's
//! path, server commands, `tilr edit` in a pane, attaching in a terminal.
//!
//! tilrd is the one next to the tilr under test (the workspace's target
//! directory), as `tilr` finds it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

#[path = "../../../tests/support/server_owner.rs"]
mod server_owner;

const TILR: &str = env!("CARGO_BIN_EXE_tilr");

/// A machine of the test's own: the runtime, state and config directories
/// of a user who runs nothing yet.
struct Machine {
    owner: server_owner::Owner,
    dir: PathBuf,
    tilr: PathBuf,
}

impl Machine {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        // Short: the socket's path must fit in sun_path (macOS: 103 bytes).
        let dir = PathBuf::from(format!(
            "/tmp/tilr-ct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            Path::new(TILR).with_file_name("tilrd").exists(),
            "tilrd is not built next to tilr: run the tests of the workspace"
        );
        Self {
            owner: server_owner::Owner::new(&dir),
            dir,
            tilr: PathBuf::from(TILR),
        }
    }

    /// Run an installation private to this test, since update replaces both
    /// programs next to one another.
    fn install_copy(&mut self) -> PathBuf {
        let bin = self.dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        self.tilr = bin.join("tilr");
        std::fs::copy(TILR, &self.tilr).unwrap();
        let server = bin.join("tilrd");
        std::fs::copy(Path::new(TILR).with_file_name("tilrd"), &server).unwrap();
        server
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("runtime/tilrd.sock")
    }

    /// The runtime directory made beforehand, private as `tilr` makes it.
    fn make_runtime_dir(&self) -> PathBuf {
        use std::os::unix::fs::DirBuilderExt;
        let dir = self.dir.join("runtime");
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        dir
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.tilr);
        command
            .args(args)
            .env("TILR_RUNTIME_DIR", self.dir.join("runtime"))
            .env("TILR_STATE_DIR", self.dir.join("state"))
            .env("TILR_CONFIG_DIR", self.dir.join("config"))
            .env("HOME", &self.dir)
            // Not the default port: the tests run at once, and next to a
            // Tilr app's server.
            .env("TILR_LISTEN", "127.0.0.1:0")
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            .env("TERM", "xterm-256color")
            .env_remove("ENV")
            .env_remove("TILR_SOCKET")
            .env_remove("TILR_CONNECT")
            .env_remove("TILR_PANE")
            .env_remove("TILR_SOCK")
            .env_remove("SSH_AUTH_SOCK")
            // The tilrd it starts logs at its default level.
            .env_remove("RUST_LOG")
            .stdin(Stdio::null());
        self.owner.configure(&mut command);
        command
    }

    fn tilr(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// `tilr ARGS`, which must succeed: its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.tilr(args);
        assert!(
            output.status.success(),
            "tilr {args:?}: {}\n{}\n--- log ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            self.log()
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// The pid of the server holding the socket's lock.
    fn server_pid(&self) -> i32 {
        let lock = tilr_proto::local::lock_path(&self.socket());
        std::fs::read_to_string(lock)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("runtime/tilrd.log")).unwrap_or_default()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        self.owner.close();
        if std::thread::panicking() {
            eprintln!("--- tilrd.log ---\n{}", self.log());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Make the socket unavailable for new connections while preserving the
/// server's open listener across a reload. The path is restored on panic too.
struct HiddenSocket {
    path: PathBuf,
    hidden: PathBuf,
    restored: bool,
}

impl HiddenSocket {
    fn hide(path: PathBuf) -> Self {
        let hidden = path.with_extension("held");
        std::fs::rename(&path, &hidden).unwrap();
        Self {
            path,
            hidden,
            restored: false,
        }
    }

    fn restore(&mut self) {
        std::fs::rename(&self.hidden, &self.path).unwrap();
        self.restored = true;
    }
}

impl Drop for HiddenSocket {
    fn drop(&mut self) {
        if !self.restored {
            let _ = std::fs::rename(&self.hidden, &self.path);
        }
    }
}

fn release_archive(server: &[u8], client: &[u8]) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (name, bytes) in [("tilrd", server), ("tilr", client)] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// A loopback CDN with a gate on the archive response. Receiving `requested`
/// proves the server's update command reached the archive before the test
/// blocks new connections to the local socket.
fn serve_release(root: PathBuf) -> (String, Receiver<()>, Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (requested_tx, requested) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split(' ').nth(1).unwrap_or("/");
            if path.ends_with(".tar.gz") {
                requested_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            let response = match std::fs::read(root.join(path.trim_start_matches('/'))) {
                Ok(body) => (
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    ),
                    body,
                ),
                Err(_) => (
                    "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        .into(),
                    Vec::new(),
                ),
            };
            let _ = stream
                .write_all(response.0.as_bytes())
                .and_then(|()| stream.write_all(&response.1));
        }
    });
    (url, requested, release)
}

fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| !stat.contains(") Z "))
        .unwrap_or(true)
}

fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it() {
    let machine = Machine::new();
    assert!(
        machine.tilr(&["ls"]).status.code() != Some(0),
        "ls starts nothing"
    );
    let first = machine.ok(&["new", "-d"]);
    let server = machine.server_pid();
    // SAFETY: getsid has no memory-safety preconditions.
    assert_eq!(
        unsafe { libc::getsid(server) },
        server,
        "a session of its own"
    );
    let parent = Command::new("ps")
        .args(["-o", "ppid=", "-p", &server.to_string()])
        .output()
        .unwrap();
    let parent: i32 = String::from_utf8_lossy(&parent.stdout)
        .trim()
        .parse()
        .unwrap();
    assert_ne!(parent, std::process::id() as i32, "not a child of the test");

    let second = machine.ok(&["new", "-d"]);
    assert_eq!(machine.server_pid(), server, "the same server");
    let listed = machine.ok(&["ls", "-q"]);
    assert_eq!(listed, format!("{first}{second}"));
    let ps = machine.ok(&["ps"]);
    assert!(ps.contains(first.trim()) && ps.contains("PID"), "{ps}");
    let version = machine.ok(&["version"]);
    assert!(
        version.starts_with("tilr ") && version.contains("\ntilrd "),
        "{version}"
    );
    // And the TCP port, on loopback, for the Tilr app and tunnels.
    assert!(
        machine.log().contains("listening on ws://127.0.0.1:"),
        "{}",
        machine.log()
    );
}

#[test]
fn a_server_tilr_starts_goes_on_without_tcp_when_the_port_is_taken() {
    // The Tilr app's server, or another user's, holds the port.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap().to_string();
    let machine = Machine::new();
    let output = machine
        .command(&["new", "-d"])
        .env("TILR_LISTEN", &addr)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        machine.log()
    );
    let log = machine.log();
    assert!(
        log.contains(&format!("{addr} is taken")) && log.contains("serving the Unix socket only"),
        "{log}"
    );
    assert!(!log.contains("listening on ws://"), "{log}");
    assert_eq!(machine.ok(&["ls", "-q"]).lines().count(), 1);
}

#[test]
fn tilr_started_many_times_at_once_starts_one_server() {
    let machine = Machine::new();
    let children: Vec<_> = (0..6)
        .map(|_| {
            machine
                .command(&["new", "-d"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut panes = Vec::new();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        panes.push(String::from_utf8(output.stdout).unwrap());
    }
    let listed = machine.ok(&["ls", "-q"]);
    for pane in &panes {
        assert!(listed.contains(pane.as_str()), "{listed}");
    }
    let log = machine.log();
    assert_eq!(
        log.matches("Unix socket for local clients").count(),
        1,
        "{log}"
    );
    assert!(
        !log.contains("another tilrd"),
        "no second server was even started: {log}"
    );
}

#[test]
fn kill_server_ends_the_shells_and_removes_the_socket() {
    let machine = Machine::new();
    let pane = machine.ok(&["new", "-d", "--", "sleep", "1000"]);
    let server = machine.server_pid();
    let ps = machine.ok(&["ps"]);
    let pane_pid: i32 = ps
        .lines()
        .find(|line| line.starts_with(pane.trim()))
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    machine.ok(&["kill-server"]);
    assert!(!alive(server), "the server is gone");
    assert!(!alive(pane_pid), "its pane process is gone");
    assert!(!machine.socket().exists());
    assert!(!tilr_proto::local::lock_path(&machine.socket()).exists());
    let again = machine.tilr(&["kill-server"]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("no tilrd runs"));
}

#[test]
fn kill_closes_the_named_pane_and_the_current_one() {
    let machine = Machine::new();
    let named = machine.ok(&["new", "-d", "--", "sleep", "1000"]);
    let current = machine.ok(&["new", "-d", "--", "sleep", "1000"]);
    let last = machine.ok(&["new", "-d", "--", "sleep", "1000"]);

    machine.ok(&["kill", named.trim()]);
    let remaining = machine.ok(&["ls", "-q"]);
    assert_eq!(remaining.lines().count(), 2);
    assert!(remaining.lines().any(|id| id == current.trim()));
    assert!(remaining.lines().any(|id| id == last.trim()));

    let output = machine
        .command(&["kill"])
        .env("TILR_PANE", current.trim())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(machine.ok(&["ls", "-q"]), last);
    machine.ok(&["kill", last.trim()]);
    assert_eq!(machine.ok(&["ls", "-q"]), "");

    let empty = machine.tilr(&["kill"]);
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no panes"));
}

#[test]
fn a_socket_left_by_a_killed_server_is_replaced_by_the_next_tilr() {
    let machine = Machine::new();
    machine.ok(&["new", "-d"]);
    let server = machine.server_pid();
    // SAFETY: kill has no memory-safety preconditions.
    unsafe { libc::kill(server, libc::SIGKILL) };
    eventually("the server dies", || !alive(server));
    assert!(machine.socket().exists(), "SIGKILL leaves the socket");
    let ls = machine.tilr(&["ls"]);
    assert!(!ls.status.success(), "ls does not start a server");
    machine.ok(&["new", "-d"]);
    assert_ne!(machine.server_pid(), server);
    assert_eq!(machine.ok(&["ls", "-q"]).lines().count(), 1);
}

#[test]
fn a_file_in_the_way_is_left_alone_with_a_clear_error() {
    let machine = Machine::new();
    machine.make_runtime_dir();
    std::fs::write(machine.socket(), "keep").unwrap();
    let output = machine.tilr(&["new", "-d"]);
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("not a socket (nothing was removed)"), "{err}");
    assert_eq!(std::fs::read_to_string(machine.socket()).unwrap(), "keep");
}

#[test]
fn a_runtime_directory_open_to_others_or_a_link_is_refused_and_nothing_started() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new();
    let runtime = machine.make_runtime_dir();
    let refused = |what: &str| {
        for args in [&["new", "-d"][..], &["ls"], &["kill-server"]] {
            let output = machine.tilr(args);
            assert!(!output.status.success(), "tilr {args:?}");
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(err.contains(what), "tilr {args:?}: {err}");
            assert!(err.contains(&runtime.display().to_string()), "{err}");
        }
        assert!(!machine.socket().exists(), "no server was started");
        assert!(machine.log().is_empty(), "no server was started");
    };

    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    refused("open to other users (mode 0755)");
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o770)).unwrap();
    refused("open to other users (mode 0770)");

    // A link to a private directory, which another user could repoint.
    let real = machine.dir.join("real");
    std::fs::rename(&runtime, &real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&real, &runtime).unwrap();
    refused("is a symbolic link");
    assert_eq!(
        std::fs::read_dir(&real).unwrap().count(),
        0,
        "nothing made there"
    );

    // Private again: it works.
    std::fs::remove_file(&runtime).unwrap();
    std::fs::rename(&real, &runtime).unwrap();
    machine.ok(&["new", "-d"]);
}

#[test]
fn a_symlinked_lock_file_is_refused() {
    let machine = Machine::new();
    let runtime = machine.make_runtime_dir();
    let target = machine.dir.join("elsewhere");
    std::fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, runtime.join("tilrd.sock.lock")).unwrap();
    let output = machine.tilr(&["new", "-d"]);
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("tilrd.sock.lock"), "{err}");
    assert!(err.contains("symbolic link is not followed"), "{err}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    assert!(!machine.socket().exists(), "no server was started");

    // The log too: a link there is not written through.
    std::fs::remove_file(runtime.join("tilrd.sock.lock")).unwrap();
    std::os::unix::fs::symlink(&target, runtime.join("tilrd.log")).unwrap();
    let output = machine.tilr(&["new", "-d"]);
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("tilrd.log"), "{err}");
    assert!(err.contains("symbolic link is not followed"), "{err}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}

#[test]
fn a_socket_path_too_long_is_a_clear_error() {
    let machine = Machine::new();
    let long = machine.dir.join("x".repeat(120)).join("tilrd.sock");
    let output = machine.tilr(&["--socket", long.to_str().unwrap(), "new", "-d"]);
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("longer than a Unix socket's"), "{err}");
}

#[test]
fn server_commands_run_in_the_server() {
    let machine = Machine::new();
    let token = machine.ok(&["token"]);
    assert_eq!(
        token,
        std::fs::read_to_string(machine.dir.join("state/token")).unwrap()
    );
    let status = machine.ok(&["cloud", "status"]);
    assert!(status.contains("tilr cloud login"), "{status}");
    let remote = machine.tilr(&["--connect", "ws://127.0.0.1:9", "token"]);
    assert!(!remote.status.success());
}

#[test]
fn tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload() {
    let mut machine = Machine::new();
    let server_exe = machine.install_copy();
    machine.ok(&["new", "-d"]);
    let server_pid = machine.server_pid();

    let version_output = Command::new(&server_exe)
        .env_remove("RUST_LOG")
        .arg("--version")
        .output()
        .unwrap();
    assert!(version_output.status.success());
    let version_line = String::from_utf8(version_output.stdout).unwrap();
    let mut words = version_line.split_whitespace();
    assert_eq!(words.next(), Some("tilrd"));
    let version = words.next().unwrap();
    let platform = words.next().unwrap().trim_matches(['(', ')']);

    let mut updated_server = std::fs::read(&server_exe).unwrap();
    updated_server.push(0); // A different executable that reports the same test build version.
    let installed_client = std::fs::read(&machine.tilr).unwrap();
    let archive = release_archive(&updated_server, &installed_client);
    let cdn_dir = machine.dir.join("cdn");
    let release_name = format!("tilr-9.9.9-test-{platform}.tar.gz");
    let archive_path = cdn_dir.join("releases/9.9.9-test").join(&release_name);
    std::fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    std::fs::write(&archive_path, &archive).unwrap();
    let checksum: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::write(
        archive_path.with_file_name(format!("{release_name}.sha256")),
        format!("{checksum}  {release_name}\n"),
    )
    .unwrap();
    let latest = cdn_dir.join("releases").join(platform);
    std::fs::create_dir_all(&latest).unwrap();
    std::fs::write(latest.join("latest"), "9.9.9-test\n").unwrap();
    let (cdn, requested, send_archive) = serve_release(cdn_dir);

    let remote = machine.tilr(&["--connect", "ws://127.0.0.1:9", "update", "--cdn", &cdn]);
    assert!(!remote.status.success());
    assert!(
        String::from_utf8_lossy(&remote.stderr)
            .contains("server commands run in the local tilrd; on another machine, run tilr there")
    );

    let mut update = machine
        .command(&["update", "--cdn", &cdn])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    requested
        .recv_timeout(Duration::from_secs(15))
        .expect("the local server requests the release archive");
    let mut hidden = HiddenSocket::hide(machine.socket());
    send_archive.send(()).unwrap();
    eventually("the updated server resumes after reload", || {
        machine.log().contains("resumed after reload with")
    });
    assert!(
        update.try_wait().unwrap().is_none(),
        "tilr update waits for a connection to the reloaded server"
    );
    hidden.restore();
    let output = update.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "tilr update: {output:?}\n--- server log ---\n{}",
        machine.log()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&format!("tilrd {version} is running")),
        "{output:?}"
    );
    assert_eq!(machine.server_pid(), server_pid);
    assert_eq!(std::fs::read(&server_exe).unwrap(), updated_server);
}

#[test]
fn tilr_edit_in_a_pane_asks_the_server_of_the_pane() {
    let machine = Machine::new();
    let sock = machine.dir.join("host.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        (&stream)
            .write_all(b"{\"ok\":true,\"clients\":1}\n")
            .unwrap();
        line
    });
    let output = machine
        .command(&["edit", "notes.md"])
        .env("TILR_PANE", "pane-1")
        .env("TILR_SOCK", &sock)
        .current_dir(&machine.dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let request: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(request["pane_id"], "pane-1");
    assert_eq!(request["kind"], "file");
    let path = std::fs::canonicalize(&machine.dir)
        .unwrap()
        .join("notes.md");
    let given = PathBuf::from(request["path"].as_str().unwrap());
    assert_eq!(
        std::fs::canonicalize(given.parent().unwrap())
            .unwrap()
            .join("notes.md"),
        path
    );
}

/// A terminal: the master side of a pty whose slave is a session's
/// controlling terminal.
struct Terminal {
    master: std::fs::File,
    child: std::process::Child,
    seen: String,
}

impl Terminal {
    fn spawn(mut command: Command) -> Self {
        let (mut master, mut slave) = (0, 0);
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty writes the two descriptors; the name, termios
        // are null (unused).
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        };
        assert_eq!(opened, 0);
        // SAFETY: openpty gave us both, owned by nothing else.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let fd = slave.as_raw_fd();
        command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: setsid and ioctl are async-signal-safe.
        unsafe {
            command.pre_exec(move || {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                let _ = fd;
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let master = std::fs::File::from(master);
        // SAFETY: fcntl on our own descriptor.
        unsafe {
            let flags = libc::fcntl(master.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Self {
            master,
            child,
            seen: String::new(),
        }
    }

    fn read(&mut self) {
        let mut buf = [0u8; 65536];
        while let Ok(n) = self.master.read(&mut buf) {
            if n == 0 {
                break;
            }
            self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }

    fn write(&mut self, text: &str) {
        self.master.write_all(text.as_bytes()).unwrap();
    }

    /// Whether the terminal is in canonical mode with echo, as `stty -a`
    /// says `icanon` and `echo`. Read on the master: once the session's
    /// leader is gone the slave descriptors are revoked (macOS).
    fn cooked(&self) -> bool {
        // SAFETY: tcgetattr writes the termios it is given.
        let mut modes: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::tcgetattr(self.master.as_raw_fd(), &raw mut modes) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        let wanted = libc::ICANON | libc::ECHO;
        modes.c_lflag & wanted == wanted
    }

    fn expect(&mut self, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.read();
            if self.seen.contains(what) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no {what:?} on the terminal: {:?}",
                &self.seen[self.seen.len().saturating_sub(2000)..]
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches() {
    let machine = Machine::new();
    let mut terminal = Terminal::spawn(machine.command(&[]));
    terminal.write("echo \"one-$((40+2))\"\r");
    terminal.expect("one-42");
    // The server reloads (as an update makes it): the client comes back by
    // itself, to the same shell.
    let server = machine.server_pid();
    // SAFETY: kill has no memory-safety preconditions.
    unsafe { libc::kill(server, libc::SIGUSR1) };
    eventually("the server reloads", || {
        machine.log().contains("resumed after reload")
    });
    // What is typed while it is away is dropped: type until it shows.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        terminal.write("echo \"two-$((40+3))\"\r");
        std::thread::sleep(Duration::from_millis(300));
        terminal.read();
        if terminal.seen.contains("two-43") {
            break;
        }
        assert!(Instant::now() < deadline, "no echo after the reload");
    }
    assert_eq!(machine.server_pid(), server);
    terminal.write("\x1dd");
    terminal.expect("[detached from pane");
    let status = terminal.child.wait().unwrap();
    assert!(status.success());
    // Still there for the next tilr.
    assert_eq!(machine.ok(&["ls", "-q"]).lines().count(), 1);
}

/// A WebSocket of the test's own to the machine's server, said `Hello` as
/// `client_id` (empty: none).
fn connect_as(
    machine: &Machine,
    client_id: &str,
) -> tokio_tungstenite::tungstenite::WebSocket<std::os::unix::net::UnixStream> {
    use tilr_proto::{ClientMessage, Hello, ServerMessage, client_message, server_message};
    use tokio_tungstenite::tungstenite::{Message, client};
    let stream = std::os::unix::net::UnixStream::connect(machine.socket()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let (mut ws, _) = client(tilr_proto::local::URL, stream).unwrap();
    let hello = client_message::Msg::Hello(Hello {
        protocol_version: tilr_proto::PROTOCOL_VERSION,
        client_name: "test".into(),
        client_id: client_id.into(),
        ..Default::default()
    });
    ws.send(Message::Binary(tilr_proto::encode(&ClientMessage::from(
        hello,
    ))))
    .unwrap();
    loop {
        let Message::Binary(packet) = ws.read().unwrap() else {
            continue;
        };
        let reply: ServerMessage = tilr_proto::decode(&packet).unwrap();
        match reply.msg {
            Some(server_message::Msg::HelloAck(_)) => return ws,
            other => assert!(
                !matches!(other, Some(server_message::Msg::Error(_))),
                "{other:?}"
            ),
        }
    }
}

/// The client id of the `tilr` viewing the machine's one pane, from the
/// server's pane list.
fn viewer_id(machine: &Machine) -> String {
    use tilr_proto::{ClientMessage, ListPanes, ServerMessage, client_message, server_message};
    use tokio_tungstenite::tungstenite::Message;
    let mut ws = connect_as(machine, "");
    let deadline = Instant::now() + Duration::from_secs(15);
    for request_id in 1.. {
        let list = client_message::Msg::ListPanes(ListPanes { request_id });
        ws.send(Message::Binary(tilr_proto::encode(&ClientMessage::from(
            list,
        ))))
        .unwrap();
        let panes = loop {
            let Message::Binary(packet) = ws.read().unwrap() else {
                continue;
            };
            let reply: ServerMessage = tilr_proto::decode(&packet).unwrap();
            if let Some(server_message::Msg::PaneList(list)) = reply.msg
                && list.request_id == request_id
            {
                break list.panes;
            }
        };
        if let Some(viewer) = panes
            .iter()
            .flat_map(|pane| &pane.viewed_by)
            .find(|viewer| viewer.client_name == "tilr")
        {
            assert!(!viewer.client_id.is_empty(), "tilr names its client id");
            return viewer.client_id.clone();
        }
        assert!(Instant::now() < deadline, "no tilr views the pane");
        std::thread::sleep(Duration::from_millis(50));
    }
    unreachable!()
}

#[test]
fn reconnects_say_the_same_client_id() {
    let machine = Machine::new();
    let mut terminal = Terminal::spawn(machine.command(&[]));
    terminal.write("echo \"client-id-$((40+2))\"\r");
    terminal.expect("client-id-42");
    let before = viewer_id(&machine);

    // The test causes the reload, then asks the replacement server who
    // views the pane. Its answer proves that tilr has reconnected.
    let server = machine.server_pid();
    // SAFETY: kill has no memory-safety preconditions.
    assert_eq!(unsafe { libc::kill(server, libc::SIGUSR1) }, 0);
    eventually("the server resumes after reload", || {
        machine.log().contains("resumed after reload")
    });
    assert_eq!(viewer_id(&machine), before);

    terminal.write("\x1dd");
    terminal.expect("[detached from pane");
    assert!(terminal.child.wait().unwrap().success());
}

#[test]
fn an_attach_superseded_by_its_own_client_id_ends() {
    let machine = Machine::new();
    let mut terminal = Terminal::spawn(machine.command(&[]));
    terminal.write("echo \"one-$((40+2))\"\r");
    terminal.expect("one-42");
    let id = viewer_id(&machine);
    // Another connection says Hello as this tilr, on the same socket: the
    // server lets tilr's go as superseded, and tilr does not take the pane
    // back.
    let _newer = connect_as(&machine, &id);
    terminal.expect("[superseded by another connection of this client]");
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        terminal.read();
        if let Some(status) = terminal.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "tilr goes on after superseded");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
    assert!(terminal.cooked(), "the terminal restored");
    assert!(
        !terminal.seen.contains("reconnecting"),
        "no reconnect: {:?}",
        &terminal.seen[terminal.seen.len().saturating_sub(2000)..]
    );
    assert_eq!(
        machine.log().matches("superseded").count(),
        1,
        "superseded once:\n{}",
        machine.log()
    );
    assert_eq!(
        machine.ok(&["ls", "-q"]).lines().count(),
        1,
        "the pane goes on"
    );
}

#[test]
fn a_terminated_attach_restores_the_terminal() {
    use std::os::unix::process::ExitStatusExt;
    let machine = Machine::new();
    let mut terminal = Terminal::spawn(machine.command(&[]));
    terminal.write("echo \"one-$((40+2))\"\r");
    terminal.expect("one-42");
    assert!(!terminal.cooked(), "raw while attached");
    // Whatever tilr was still drawing, read: a terminal reads as it comes.
    std::thread::sleep(Duration::from_millis(300));
    terminal.read();
    let seen = terminal.seen.len();
    // SAFETY: kill has no memory-safety preconditions.
    unsafe { libc::kill(terminal.child.id() as i32, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        terminal.read();
        if let Some(status) = terminal.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "tilr goes on after SIGTERM");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{status:?}");
    assert!(terminal.cooked(), "icanon and echo again");
    terminal.read();
    let after = &terminal.seen[seen..];
    assert!(
        after.contains("\x1b[?25h") && after.contains("\x1b[?1049l"),
        "cursor shown, main screen: {after:?}"
    );
    assert!(!after.contains("detached"), "nothing printed: {after:?}");
    assert_eq!(
        machine.ok(&["ls", "-q"]).lines().count(),
        1,
        "the pane goes on"
    );
}

/// The pane a [`FakeServer`] has.
const FAKE_PANE: &str = "fake-pane-0123456789";

/// How a [`FakeServer`] serves one connection.
#[derive(Clone, Copy)]
enum Serve {
    /// Hello, the pane list, a frame on Subscribe; then it closes.
    ThenClose,
    /// Hello, then it closes on `ListPanes` without answering.
    CloseOnListPanes,
    /// Accepts and says nothing, not even the WebSocket handshake.
    Silent,
    /// Everything, for as long as the client stays.
    Fully,
}

/// A stand-in for tilrd on a TCP port: its connections served in turn as
/// `script` says, the last way for any after that.
struct FakeServer {
    url: String,
    accepted: std::sync::Arc<AtomicU32>,
    /// The text of the keys it was sent.
    keys: std::sync::mpsc::Receiver<String>,
}

impl FakeServer {
    fn start(script: Vec<Serve>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let accepted = std::sync::Arc::new(AtomicU32::new(0));
        let (key_tx, keys) = std::sync::mpsc::channel();
        let counter = accepted.clone();
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().enumerate() {
                let Ok(stream) = stream else { return };
                let serve = script[n.min(script.len() - 1)];
                counter.fetch_add(1, Ordering::SeqCst);
                let keys = key_tx.clone();
                std::thread::spawn(move || fake_connection(stream, serve, n + 1, &keys));
            }
        });
        Self {
            url,
            accepted,
            keys,
        }
    }

    fn accepted(&self) -> u32 {
        self.accepted.load(Ordering::SeqCst)
    }
}

fn fake_connection(
    stream: std::net::TcpStream,
    serve: Serve,
    number: usize,
    keys: &std::sync::mpsc::Sender<String>,
) {
    use tilr_proto::{
        ClientMessage, Frame, HelloAck, PaneInfo, PaneList, Row, Run, ServerMessage,
        client_message, server_message,
    };
    use tokio_tungstenite::tungstenite::{Message, accept};
    if let Serve::Silent = serve {
        std::thread::sleep(Duration::from_secs(60));
        drop(stream);
        return;
    }
    let Ok(mut ws) = accept(stream) else { return };
    let send = |ws: &mut tokio_tungstenite::tungstenite::WebSocket<_>, msg| {
        ws.send(Message::Binary(tilr_proto::encode(&ServerMessage::from(
            msg,
        ))))
    };
    while let Ok(message) = ws.read() {
        let Message::Binary(packet) = message else {
            continue;
        };
        let Ok(ClientMessage { msg: Some(msg), .. }) = tilr_proto::decode(&packet) else {
            continue;
        };
        let sent = match msg {
            client_message::Msg::Hello(_) => send(
                &mut ws,
                server_message::Msg::HelloAck(HelloAck {
                    protocol_version: tilr_proto::PROTOCOL_VERSION,
                    server_version: "fake".into(),
                    ..Default::default()
                }),
            ),
            client_message::Msg::ListPanes(_) if matches!(serve, Serve::CloseOnListPanes) => {
                return;
            }
            client_message::Msg::ListPanes(list) => send(
                &mut ws,
                server_message::Msg::PaneList(PaneList {
                    panes: vec![PaneInfo {
                        pane_id: FAKE_PANE.into(),
                        cols: 80,
                        rows: 24,
                        ..Default::default()
                    }],
                    request_id: list.request_id,
                }),
            ),
            client_message::Msg::Subscribe(_) => {
                let frame = send(
                    &mut ws,
                    server_message::Msg::Frame(Frame {
                        pane_id: FAKE_PANE.into(),
                        full: true,
                        cols: 80,
                        rows: 24,
                        changed: vec![Row {
                            y: 0,
                            runs: vec![Run {
                                text: format!("fake screen {number}"),
                                ..Default::default()
                            }],
                        }],
                        ..Default::default()
                    }),
                );
                if let Serve::ThenClose = serve {
                    std::thread::sleep(Duration::from_millis(200));
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    return;
                }
                frame
            }
            client_message::Msg::Key(key) => {
                let _ = keys.send(key.text);
                Ok(())
            }
            _ => Ok(()),
        };
        if sent.is_err() {
            return;
        }
    }
}

/// `tilr attach` of the fake server's pane, in a terminal.
fn attach_to_fake(machine: &Machine, server: &FakeServer) -> Terminal {
    let mut command = machine.command(&[
        "--connect",
        &server.url,
        "--token",
        "fake-token",
        "attach",
        FAKE_PANE,
    ]);
    command.env_remove("TILR_TOKEN");
    let mut terminal = Terminal::spawn(command);
    terminal.expect("fake screen 1");
    terminal
}

#[test]
fn an_error_right_after_a_reconnect_is_retried() {
    let machine = Machine::new();
    let server = FakeServer::start(vec![
        Serve::ThenClose,
        Serve::CloseOnListPanes,
        Serve::Fully,
    ]);
    let mut terminal = attach_to_fake(&machine, &server);
    // The second connection is lost before the pane list comes: another
    // attempt, not the end. The status line says so a second time (the
    // terminal cuts it at 80 columns, before the attempt's number).
    terminal.expect("fake screen 3");
    assert_eq!(
        terminal.seen.matches("tilr: disconnected (").count(),
        2,
        "{:?}",
        terminal.seen
    );
    terminal.write("x");
    let key = server.keys.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(key, "x");
    terminal.write("\x1dd");
    terminal.expect("[detached from pane");
    assert!(terminal.child.wait().unwrap().success());
    assert_eq!(server.accepted(), 3);
}

#[test]
fn detach_works_while_a_reconnect_attempt_hangs() {
    let machine = Machine::new();
    let server = FakeServer::start(vec![Serve::ThenClose, Serve::Silent]);
    let mut terminal = attach_to_fake(&machine, &server);
    eventually("tilr connects again", || server.accepted() == 2);
    // Well inside the attempt, which would wait 5 s for an answer.
    std::thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    terminal.write("\x1dd");
    terminal.expect("[detached from pane");
    let took = start.elapsed();
    assert!(took < Duration::from_secs(1), "detached after {took:?}");
    assert!(terminal.child.wait().unwrap().success());
}

#[test]
fn cloud_login_help_names_the_default_of_tilrd() {
    let default = |program: &Path| {
        let output = Command::new(program)
            .args(["cloud", "login", "--help"])
            .env_remove("TILR_CLOUD")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        help.lines()
            .find_map(|line| {
                let (_, rest) = line.split_once("[default: ")?;
                Some(rest.split(']').next()?.to_owned())
            })
            .unwrap_or_else(|| panic!("no default in {help}"))
    };
    let tilr = default(Path::new(TILR));
    assert_eq!(tilr, "https://wss.tilr.dev");
    assert_eq!(tilr, default(&Path::new(TILR).with_file_name("tilrd")));
}

#[test]
fn a_panicking_test_stops_its_copied_daemon_without_a_command_socket() {
    let mut machine = Machine::new();
    machine.install_copy();
    machine.ok(&["new", "-d"]);
    let pid = machine.server_pid();
    let watcher = server_owner::watch(pid as u32).expect("running daemon");
    std::fs::remove_file(machine.socket()).unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _machine = machine;
        panic!("controlled client test panic");
    }));
    let exited = server_owner::wait(&watcher);
    if !exited {
        // SAFETY: the test started this exact daemon and registered its exit.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        assert!(server_owner::wait(&watcher), "failed regression cleanup");
    }
    assert!(panic.is_err());
    assert!(
        exited,
        "the copied daemon survived its test without a command socket"
    );
}
