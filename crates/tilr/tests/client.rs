//! Use cases of `tilr` with a real tilrd: starting the server tmux-style,
//! one server for many clients, kill-server, what is found at the socket's
//! path, server commands, `tilr edit` in a pane, attaching in a terminal.
//!
//! tilrd is the one next to the tilr under test (the workspace's target
//! directory), as `tilr` finds it.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const TILR: &str = env!("CARGO_BIN_EXE_tilr");

/// A machine of the test's own: the runtime, state and config directories
/// of a user who runs nothing yet.
struct Machine {
    dir: PathBuf,
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
        Self { dir }
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
        let mut command = Command::new(TILR);
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
            .stdin(Stdio::null());
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
        if self.socket().exists() {
            let _ = self.tilr(&["kill-server"]);
        }
        if std::thread::panicking() {
            eprintln!("--- tilrd.log ---\n{}", self.log());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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
