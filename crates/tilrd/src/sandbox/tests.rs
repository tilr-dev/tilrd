//! The sandbox below the bus: what it refuses, the environment and the leftovers on
//! every platform; on Linux real sandboxed programs on a pty, checked for what they
//! can and cannot do. The bus-level tests (`tests/sandbox.rs`) start panes from
//! profiles.

use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::*;

thread_local! {
    /// The Landlock ABI a test builds its sandboxes for, as the failpoint `sandbox-abi-4`
    /// makes a server see.
    pub(super) static ABI: Cell<Option<u32>> = const { Cell::new(None) };
    /// The failpoint `sandbox-apply` for this thread's sandboxes.
    pub(super) static FAIL_APPLY: Cell<bool> = const { Cell::new(false) };
}

/// A directory tree of a test: `w` (granted read-write), `r` (read-only), `o` (the
/// user's, not granted), and the server's runtime, state and configuration directories.
struct Tree {
    root: PathBuf,
    dirs: Dirs,
}

impl Tree {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("tilrd-sandbox-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["w", "r", "o", "x", "runtime", "state", "config"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let root = std::fs::canonicalize(&root).unwrap();
        let mut dirs = Dirs {
            runtime: root.join("runtime"),
            protected: Vec::new(),
        };
        dirs.protect(root.join("runtime"), "the runtime directory (sockets)");
        dirs.protect(root.join("state"), "the state directory (token, root key)");
        dirs.protect(
            root.join("config"),
            "the configuration directory (profiles, bus.toml)",
        );
        Tree { root, dirs }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    #[cfg(target_os = "linux")]
    fn text(&self, name: &str) -> String {
        self.path(name).display().to_string()
    }

    /// A request granting `w` read-write and `r` read-only.
    #[cfg(target_os = "linux")]
    fn request(&self, network: Network) -> Request {
        Request {
            read_only: vec![self.path("r")],
            read_write: vec![self.path("w")],
            network,
            ..Request::default()
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_grant_covering_a_server_directory_is_refused() {
    let tree = Tree::new("refused");
    let refuse = |path: PathBuf| refuse_exposing([path.as_path()], &tree.dirs);
    // Over the state directory (a home granted when the state is under it).
    let err = refuse(tree.root.clone()).unwrap_err();
    assert!(err.starts_with("sandbox: "), "{err}");
    assert!(err.contains("would expose the"), "{err}");
    // The directory itself, one inside it, and the same through `..` or a link.
    assert!(
        refuse(tree.path("config"))
            .unwrap_err()
            .contains("configuration directory")
    );
    assert!(
        refuse(tree.path("state/keys"))
            .unwrap_err()
            .contains("state directory")
    );
    assert!(
        refuse(tree.path("w/../runtime"))
            .unwrap_err()
            .contains("runtime directory")
    );
    std::os::unix::fs::symlink(tree.path("state"), tree.path("w/link")).unwrap();
    assert!(
        refuse(tree.path("w/link"))
            .unwrap_err()
            .contains("state directory")
    );
    // Beside them, and a relative path.
    refuse(tree.path("w")).unwrap();
    refuse(tree.path("statement")).unwrap();
    assert!(
        refuse(PathBuf::from("w"))
            .unwrap_err()
            .contains("not an absolute path")
    );
}

#[test]
fn a_sandboxed_pane_gets_only_its_variables() {
    let request = Request {
        pass_env: vec![
            "LANG".into(),
            "TILR_SHIMS".into(),
            "SSH_AUTH_SOCK".into(),
            "UNSET".into(),
        ],
        env: vec![
            ("GREETING".into(), "hi".into()),
            ("TERM".into(), "dumb".into()),
        ],
        ..Request::default()
    };
    let server = |name: &str| match name {
        "LANG" => Some("C.UTF-8".into()),
        "SECRET" => Some("x".into()),
        "TILR_SHIMS" => Some("/run/tilr/bin".into()),
        "SSH_AUTH_SOCK" => Some("/run/agent.sock".into()),
        _ => None,
    };
    let base = vec![
        ("HOME".to_owned(), OsString::from("/home/u")),
        ("TERM".to_owned(), OsString::from("xterm-256color")),
        ("TILR_PANE".to_owned(), OsString::from("p1")),
    ];
    let exes = [
        PathBuf::from("/opt/tool/bin/tool"),
        PathBuf::from("/opt/tool/bin/other"),
    ];
    let env = environment(
        &request,
        &base,
        Path::new("/run/tilr/sandbox/p1"),
        &exes,
        &server,
    );
    let env: Vec<(String, String)> = env
        .into_iter()
        .map(|(n, v)| (n.into_string().unwrap(), v.into_string().unwrap()))
        .collect();
    let get = |name: &str| env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    assert_eq!(get("LANG"), Some("C.UTF-8"));
    assert_eq!(get("GREETING"), Some("hi"));
    // The profile's [env] is set last.
    assert_eq!(get("TERM"), Some("dumb"));
    assert_eq!(get("TMPDIR"), Some("/run/tilr/sandbox/p1"));
    assert_eq!(
        get("PATH"),
        Some("/opt/tool/bin:/usr/bin:/bin:/usr/sbin:/sbin")
    );
    for never in ["SECRET", "TILR_SHIMS", "SSH_AUTH_SOCK", "UNSET"] {
        assert_eq!(get(never), None, "{never}");
    }
    let names: Vec<&str> = env.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "HOME",
            "TILR_PANE",
            "TMPDIR",
            "PATH",
            "LANG",
            "GREETING",
            "TERM"
        ]
    );
}

#[test]
fn a_sandboxed_panes_directory_is_private_and_removed() {
    let tree = Tree::new("leftovers");
    let dir = create_pane_dir(&tree.dirs.runtime, "p1").unwrap();
    assert_eq!(dir, pane_dir(&tree.dirs.runtime, "p1"));
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    std::fs::write(dir.join("x"), "x").unwrap();
    remove_pane_dir(&tree.dirs.runtime, "p1");
    assert!(!dir.exists());
    // At start: what panes that no longer run left goes, a running one's stays.
    for id in ["gone", "running"] {
        create_pane_dir(&tree.dirs.runtime, id).unwrap();
    }
    remove_leftovers(&tree.dirs.runtime, |id| id == "running");
    assert!(!pane_dir(&tree.dirs.runtime, "gone").exists());
    assert!(pane_dir(&tree.dirs.runtime, "running").exists());
    assert!(create_pane_dir(&tree.dirs.runtime, "../x").is_err());
}

#[test]
fn a_request_crosses_the_command_line_whole() {
    let request = Request {
        read_only: vec!["/a".into()],
        read_write: vec!["/b".into()],
        executables: vec!["git".into(), "/c".into()],
        pass_env: vec!["LANG".into()],
        network: Network::Client,
        env: vec![("A".into(), "1".into())],
    };
    assert_eq!(Request::from_arg(&request.to_arg()).unwrap(), request);
    assert!(
        Request::from_arg("{\"read_onlyy\":[]}")
            .unwrap_err()
            .starts_with("sandbox: ")
    );
}

#[test]
fn a_kernel_without_signal_scoping_is_named_in_a_warning() {
    let warning = scoping_warning(Features { landlock_abi: 4 }, "p1");
    if cfg!(target_os = "linux") {
        assert_eq!(
            warning.as_deref(),
            Some(
                "sandbox: kernel lacks signal scoping (Landlock ABI 4 < 6); pane p1 can signal \
                 the user's other processes"
            )
        );
    }
    assert_eq!(scoping_warning(Features { landlock_abi: 6 }, "p1"), None);
}

/// Real sandboxed programs on a pty.
#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    /// How long a sandboxed program gets to do what a test asks; it is bounded so that
    /// a regression fails instead of hanging.
    const LIMIT: Duration = Duration::from_secs(20);

    /// A sandboxed program on a pty, its output collected.
    struct Term {
        master: OwnedFd,
        child: tokio::process::Child,
        chunks: mpsc::Receiver<Vec<u8>>,
        out: String,
        /// Where in `out` the next [`expect`](Term::expect) starts looking.
        seen: usize,
    }

    fn start(tree: &Tree, request: &Request, command: &[&str]) -> anyhow::Result<Term> {
        let command: Vec<String> = command.iter().map(|arg| (*arg).to_owned()).collect();
        let spawned = crate::pty::spawn_with(
            "p1",
            &command,
            &tree.text("w"),
            80,
            24,
            Some((request, &tree.dirs)),
        )?;
        let reader = spawned.master.try_clone().unwrap();
        let (tx, chunks) = mpsc::channel();
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(reader);
            let mut buf = [0u8; 4096];
            // EOF or EIO: every process holding the terminal is gone.
            while let Ok(n @ 1..) = file.read(&mut buf) {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Term {
            master: spawned.master,
            child: spawned.child,
            chunks,
            out: String::new(),
            seen: 0,
        })
    }

    impl Term {
        fn write(&self, text: &str) {
            crate::pty::write(self.master.as_raw_fd(), text.as_bytes()).unwrap();
        }

        /// Wait for `needle` in what comes after the last match; return what came
        /// before it.
        fn expect(&mut self, needle: &str) -> String {
            let deadline = Instant::now() + LIMIT;
            loop {
                if let Some(at) = self.out[self.seen..].find(needle) {
                    let before = self.out[self.seen..self.seen + at].to_owned();
                    self.seen += at + needle.len();
                    return before;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                match self.chunks.recv_timeout(left) {
                    Ok(chunk) => self
                        .out
                        .push_str(&String::from_utf8_lossy(&chunk).replace('\r', "")),
                    Err(err) => panic!("{err:?} waiting for {needle:?}; output:\n{}", self.out),
                }
            }
        }

        /// Wait for the program to exit and for the end of its output.
        async fn finish(mut self) -> (Option<i32>, String) {
            let status = tokio::time::timeout(LIMIT, self.child.wait())
                .await
                .unwrap_or_else(|_| panic!("the program did not exit; output:\n{}", self.out))
                .unwrap();
            let deadline = Instant::now() + LIMIT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.chunks.recv_timeout(left) {
                    Ok(chunk) => self
                        .out
                        .push_str(&String::from_utf8_lossy(&chunk).replace('\r', "")),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(err) => panic!("{err:?}: the terminal stays open; output:\n{}", self.out),
                }
            }
            (status.code(), self.out)
        }
    }

    /// Run `script` with `/bin/sh -c` in a sandbox; its exit code and output.
    async fn run(tree: &Tree, request: &Request, script: &str) -> (Option<i32>, String) {
        start(tree, request, &["/bin/sh", "-c", script])
            .unwrap()
            .finish()
            .await
    }

    fn copy_program(to: &Path) {
        std::fs::copy("/bin/echo", to).unwrap();
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn a_sandboxed_program_writes_only_where_it_may() {
        let tree = Tree::new("writes");
        std::fs::write(tree.path("r/readable"), "ro-content\n").unwrap();
        std::fs::write(tree.path("o/secret"), "SECRET\n").unwrap();
        let (w, r, o) = (tree.text("w"), tree.text("r"), tree.text("o"));
        let script =
            format!("touch {w}/a {r}/b {o}/c; cat {r}/readable {o}/secret; ls {o}; echo END");
        let (code, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert_eq!(code, Some(0), "{out}");
        assert!(tree.path("w/a").exists(), "{out}");
        assert!(
            !tree.path("r/b").exists() && !tree.path("o/c").exists(),
            "{out}"
        );
        assert!(out.contains("ro-content"), "{out}");
        assert!(!out.contains("SECRET"), "{out}");
        assert_eq!(out.matches("Permission denied").count(), 4, "{out}");
        assert!(out.ends_with("END\n"), "{out}");
    }

    #[tokio::test]
    async fn a_child_of_the_sandboxed_program_is_restricted_too() {
        let tree = Tree::new("grandchild");
        let o = tree.text("o");
        let script = format!("sh -c 'sh -c \"touch {o}/d\"'; echo rc=$?");
        let (_, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert!(
            out.contains("Permission denied") && out.contains("rc=1"),
            "{out}"
        );
        assert!(!tree.path("o/d").exists());
    }

    #[tokio::test]
    async fn only_granted_programs_run_in_a_sandbox() {
        let tree = Tree::new("programs");
        copy_program(&tree.path("o/tool"));
        copy_program(&tree.path("x/granted"));
        copy_program(&tree.path("x/byname"));
        let mut request = tree.request(Network::None);
        request.executables = vec![tree.path("x/granted"), "byname".into(), "missing".into()];
        // `byname` is looked up in the PATH of the profile's [env].
        request.env = vec![(
            "PATH".into(),
            format!("/nowhere:{}:/usr/bin:/bin", tree.text("x")),
        )];
        let (o, x) = (tree.text("o"), tree.text("x"));
        let script =
            format!("{o}/tool A; echo rc=$?; {x}/granted B; byname C; ls {x} || echo nolist");
        let (_, out) = run(&tree, &request, &script).await;
        assert!(
            out.contains("Permission denied") && out.contains("rc=126"),
            "{out}"
        );
        assert!(!out.contains("A\n"), "{out}");
        assert!(out.contains("B\nC\n"), "{out}");
        // A granted program is not a granted directory.
        assert!(out.contains("nolist"), "{out}");
    }

    #[tokio::test]
    async fn a_script_and_a_binary_made_inside_run_inside() {
        let tree = Tree::new("inside");
        let w = tree.text("w");
        let script = format!(
            "printf '#!/bin/sh\\necho ok\\n' > {w}/t.sh && chmod +x {w}/t.sh && {w}/t.sh && \
             cp /bin/echo {w}/e && {w}/e hi"
        );
        let (code, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert_eq!(code, Some(0), "{out}");
        assert_eq!(out, "ok\nhi\n");
    }

    /// A Python snippet run in a sandbox (`python3 -I`), its output.
    async fn python(tree: &Tree, request: &Request, code: &str) -> String {
        let (_, out) = start(tree, request, &["python3", "-I", "-c", code])
            .unwrap()
            .finish()
            .await;
        out
    }

    /// Try `what`, printing `<label> ok` or `<label> <errno name>`.
    const TRY: &str = "import errno, socket, sys\n\
        def attempt(label, what):\n\
        \x20   try:\n\
        \x20       what()\n\
        \x20       print(label, 'ok', flush=True)\n\
        \x20   except OSError as err:\n\
        \x20       print(label, errno.errorcode.get(err.errno, err.errno), flush=True)\n";

    #[tokio::test]
    async fn network_follows_the_request() {
        let tree = Tree::new("network");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let tries = format!(
            "{TRY}\
             attempt('connect', lambda: socket.create_connection(('127.0.0.1', {port}), 5))\n\
             attempt('bind', lambda: socket.socket().bind(('127.0.0.1', 0)))\n\
             def listen():\n\
             \x20   s = socket.socket(); s.listen()\n\
             attempt('listen', listen)\n\
             attempt('udp', lambda: socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(b'x', ('127.0.0.1', {port})))\n\
             attempt('sctp', lambda: socket.socket(socket.AF_INET, socket.SOCK_STREAM, 132))\n"
        );

        let out = python(&tree, &tree.request(Network::None), &tries).await;
        assert!(out.contains("connect EACCES"), "{out}");
        assert!(
            out.contains("bind EACCES") && out.contains("listen EACCES"),
            "{out}"
        );
        assert!(out.contains("udp EACCES"), "{out}");
        // The listener saw nothing: the program has exited, so a connection would wait.
        assert!(listener.accept().is_err());

        let out = python(&tree, &tree.request(Network::Client), &tries).await;
        assert!(out.contains("connect ok"), "{out}");
        assert!(listener.accept().is_ok());
        assert!(
            out.contains("bind EACCES") && out.contains("listen EACCES"),
            "{out}"
        );
        // UDP is not restricted with `client` (a limit); SCTP is not TCP.
        assert!(out.contains("udp ok"), "{out}");
        assert!(out.contains("sctp EACCES"), "{out}");

        let serve = "import socket\n\
            s = socket.socket(); s.bind(('127.0.0.1', 0)); s.listen()\n\
            print('port', s.getsockname()[1], flush=True)\n\
            c, _ = s.accept(); print('accepted', flush=True)\n";
        let mut term = start(
            &tree,
            &tree.request(Network::Server),
            &["python3", "-I", "-c", serve],
        )
        .unwrap();
        term.expect("port ");
        let port: u16 = term.expect("\n").trim().parse().unwrap();
        let _connection = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        term.expect("accepted");
        let (code, out) = term.finish().await;
        assert_eq!(code, Some(0), "{out}");
    }

    #[tokio::test]
    async fn a_sandboxed_program_cannot_reach_the_server() {
        let tree = Tree::new("server");
        std::fs::write(tree.path("state/token"), "secret-token\n").unwrap();
        // A Unix socket in a directory the program may write, so only the socket rule
        // stands in its way; the server's sockets are no different.
        let socket = tree.path("w/server.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let code = format!(
            "{TRY}\
             attempt('unix', lambda: socket.socket(socket.AF_UNIX).connect({:?}))\n\
             attempt('pair', lambda: socket.socketpair())\n\
             attempt('token', lambda: open({:?}).read())\n",
            socket.display().to_string(),
            tree.text("state/token"),
        );
        let out = python(&tree, &tree.request(Network::Server), &code).await;
        assert!(out.contains("unix EACCES"), "{out}");
        assert!(out.contains("pair ok"), "{out}");
        assert!(out.contains("token EACCES"), "{out}");
        assert!(listener.accept().is_err(), "the server saw a connection");
        remove_pane_dir(&tree.dirs.runtime, "p1");
        // A grant over the server's directories never starts.
        let mut request = tree.request(Network::None);
        request.read_only.push(tree.root.clone());
        let err = start(&tree, &request, &["/bin/true"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.starts_with("sandbox: ") && err.contains("would expose"),
            "{err}"
        );
        assert!(!pane_dir(&tree.dirs.runtime, "p1").exists());
    }

    #[tokio::test]
    async fn a_sandboxed_program_has_only_its_variables() {
        let tree = Tree::new("env");
        let mut request = tree.request(Network::None);
        request.env = vec![("GREETING".into(), "hi".into())];
        let (code, out) = run(&tree, &request, "env").await;
        assert_eq!(code, Some(0), "{out}");
        // What `sh` exports of its own is not handed down.
        let shells = ["PWD", "OLDPWD", "SHLVL", "_"];
        let mut names: Vec<&str> = out
            .lines()
            .filter_map(|line| line.split('=').next())
            .filter(|name| !shells.contains(name))
            .collect();
        names.sort_unstable();
        let expected = [
            "COLORTERM",
            "GREETING",
            "HOME",
            "LOGNAME",
            "PATH",
            "SHELL",
            "TERM",
            "TILR_PANE",
            "TMPDIR",
            "USER",
        ];
        assert_eq!(names, expected, "{out}");
        assert!(
            out.contains("PATH=/usr/bin:/bin:/usr/sbin:/sbin\n"),
            "{out}"
        );
        let tmpdir = pane_dir(&tree.dirs.runtime, "p1");
        assert!(
            out.contains(&format!("TMPDIR={}\n", tmpdir.display())),
            "{out}"
        );
    }

    #[tokio::test]
    async fn the_private_tmpdir_is_writable() {
        let tree = Tree::new("tmpdir");
        let (code, out) = run(
            &tree,
            &tree.request(Network::None),
            "echo x > $TMPDIR/x && cat $TMPDIR/x",
        )
        .await;
        assert_eq!((code, out.as_str()), (Some(0), "x\n"));
        let dir = pane_dir(&tree.dirs.runtime, "p1");
        assert!(dir.join("x").exists());
        // The pane process removes it when the pane ends.
        remove_pane_dir(&tree.dirs.runtime, "p1");
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn a_sandboxed_pane_cannot_open_another_terminal() {
        let tree = Tree::new("terminals");
        // Another terminal of the same user: another pane's, say.
        let (_other_master, other_slave) = crate::pty::open_pty(80, 24).unwrap();
        let other = std::fs::read_link(format!("/proc/self/fd/{}", other_slave.as_raw_fd()))
            .unwrap()
            .display()
            .to_string();
        let script = format!(
            "(: < {other}) 2>/dev/null && echo read-other || echo no-read-other; \
             (: > {other}) 2>/dev/null && echo write-other || echo no-write-other; \
             own=$(tty); echo own > $own && echo own-ok; echo tty > /dev/tty && echo tty-ok"
        );
        let (_, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert!(
            out.contains("no-read-other") && out.contains("no-write-other"),
            "{out}"
        );
        assert!(out.contains("own-ok") && out.contains("tty-ok"), "{out}");
    }

    /// A terminal a program makes inside a sandboxed pane: its master opens
    /// (`/dev/ptmx`), its slave does not, by path or by `TIOCGPTPEER` (Landlock checks
    /// the open of `/dev/pts/M` either way), so `openpty`, `script` and tmux fail. The
    /// price of granting only the pane's own terminal.
    #[tokio::test]
    async fn a_terminal_made_inside_a_sandbox_cannot_be_opened() {
        let tree = Tree::new("inner");
        let code = "import ctypes, fcntl, os\n\
            libc = ctypes.CDLL(None, use_errno=True); libc.ptsname.restype = ctypes.c_char_p\n\
            def attempt(label, what):\n\
            \x20   try:\n\
            \x20       what(); print(label, 'ok', flush=True)\n\
            \x20   except OSError as err:\n\
            \x20       print(label, 'errno', err.errno, flush=True)\n\
            m = os.open('/dev/ptmx', os.O_RDWR | os.O_NOCTTY); libc.grantpt(m); libc.unlockpt(m)\n\
            print('master ok', flush=True)\n\
            attempt('peer', lambda: fcntl.ioctl(m, 0x5441, os.O_RDWR | os.O_NOCTTY))\n\
            attempt('path', lambda: os.open(libc.ptsname(m), os.O_RDWR | os.O_NOCTTY))\n\
            attempt('openpty', os.openpty)\n";
        let (_, out) = start(
            &tree,
            &tree.request(Network::None),
            &["python3", "-I", "-c", code],
        )
        .unwrap()
        .finish()
        .await;
        assert!(out.contains("master ok"), "{out}");
        for label in ["peer", "path", "openpty"] {
            assert!(out.contains(&format!("{label} errno 13")), "{out}");
        }
    }

    #[tokio::test]
    async fn a_sandbox_that_cannot_be_applied_starts_nothing() {
        let tree = Tree::new("apply");
        FAIL_APPLY.set(true);
        let marker = tree.path("w/marker");
        let script = format!("touch {}", marker.display());
        let err = start(
            &tree,
            &tree.request(Network::None),
            &["/bin/sh", "-c", &script],
        )
        .err()
        .expect("the program started");
        FAIL_APPLY.set(false);
        let err = err.to_string();
        assert!(err.starts_with("sandbox: "), "{err}");
        assert!(!marker.exists());
        assert!(!pane_dir(&tree.dirs.runtime, "p1").exists());
    }

    #[tokio::test]
    async fn signals_follow_the_kernels_scoping() {
        let tree = Tree::new("signals");
        let mut outside = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let script = format!("kill -0 {} && echo reached || echo refused", outside.id());
        let features = features().expect("this host offers the sandbox");
        let (_, out) = run(&tree, &tree.request(Network::None), &script).await;
        if features.signal_scoping() {
            assert!(out.contains("refused"), "{out}");
        } else {
            assert!(out.contains("reached"), "{out}");
        }
        // On a kernel without scoping (or made to look like one), the pane still starts.
        ABI.set(Some(4));
        let (code, out) = run(&tree, &tree.request(Network::None), &script).await;
        ABI.set(None);
        assert_eq!(code, Some(0), "{out}");
        assert!(out.contains("reached"), "{out}");
        outside.kill().unwrap();
        outside.wait().unwrap();
    }

    /// An interactive bash in a sandbox, at its prompt.
    fn bash(tree: &Tree) -> Term {
        let mut request = tree.request(Network::None);
        request.env = vec![("PS1".into(), "tilr$ ".into())];
        let mut term = start(
            tree,
            &request,
            &["/bin/bash", "--norc", "--noprofile", "-i"],
        )
        .unwrap();
        term.expect("tilr$ ");
        term
    }

    #[tokio::test]
    async fn raw_mode_and_window_size_work_in_a_sandbox() {
        let tree = Tree::new("raw");
        let mut term = bash(&tree);
        // `RE""ADY` in the echoed command line does not match `READY`.
        term.write("stty raw -echo; echo RE\"\"ADY; dd bs=1 count=1 2>/dev/null | od -c; stty sane; echo DO\"\"NE\r");
        term.expect("READY");
        term.write("x");
        let raw = term.expect("DONE");
        assert!(raw.contains("0000000   x"), "{raw}");
        crate::pty::set_window_size(term.master.as_fd(), 100, 30).unwrap();
        term.write("stty size; echo SI\"\"ZE\r");
        let size = term.expect("SIZE");
        assert!(size.contains("30 100"), "{size}");
        term.write("exit 0\r");
        let (code, out) = term.finish().await;
        assert_eq!(code, Some(0), "{out}");
        assert!(!out.contains("stty:"), "{out}");
    }

    #[tokio::test]
    async fn job_control_works_in_a_sandbox() {
        let tree = Tree::new("jobs");
        std::fs::write(
            tree.path("r/job.py"),
            "import signal\n\
             signal.signal(signal.SIGCONT, lambda *_: print('RESUMED', flush=True))\n\
             print('STARTED', flush=True)\n\
             while True:\n    signal.pause()\n",
        )
        .unwrap();
        let mut term = bash(&tree);
        term.write(&format!("python3 -I {}\r", tree.text("r/job.py")));
        term.expect("STARTED");
        term.write("\x1a");
        term.expect("Stopped");
        term.write("jobs; echo JO\"\"BS\r");
        let jobs = term.expect("JOBS");
        assert!(
            jobs.contains("Stopped") && jobs.contains("job.py"),
            "{jobs}"
        );
        term.write("fg\r");
        term.expect("RESUMED");
        term.write("\x03");
        term.expect("KeyboardInterrupt");
        term.expect("tilr$ ");
        term.write("exit 0\r");
        let (code, out) = term.finish().await;
        assert_eq!(code, Some(0), "{out}");
        for bad in ["process group", "no job control", "Inappropriate ioctl"] {
            assert!(!out.contains(bad), "{bad}: {out}");
        }
    }

    #[tokio::test]
    async fn a_full_screen_editor_works_in_a_sandbox() {
        let tree = Tree::new("editor");
        let file = tree.text("w/f");
        let mut term = start(
            &tree,
            &tree.request(Network::None),
            &["vi", "-i", "NONE", &file],
        )
        .expect("vi is installed");
        // vi has drawn the empty file: it reads keys now.
        term.expect("[New");
        term.write("ihello\x1b:wq\r");
        let (code, out) = term.finish().await;
        assert_eq!(code, Some(0), "{out}");
        assert_eq!(
            std::fs::read_to_string(tree.path("w/f")).unwrap(),
            "hello\n"
        );
    }
}
