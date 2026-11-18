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
            write_protected: Vec::new(),
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
    let refuse = |path: PathBuf| refuse_exposing([(path.as_path(), false)], &tree.dirs);
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
            ("TILR_SOCKET".into(), "/elsewhere.sock".into()),
            ("SSH_AUTH_SOCK".into(), "/agent.sock".into()),
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
    // Neither from the server nor from the profile's [env].
    for never in [
        "SECRET",
        "TILR_SHIMS",
        "TILR_SOCKET",
        "SSH_AUTH_SOCK",
        "UNSET",
    ] {
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
fn the_servers_executable_and_configuration_are_protected() {
    let tree = Tree::new("own");
    // This process's executable is the server's (the pane process is the same file).
    let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    assert!(
        Dirs::current()
            .write_protected
            .iter()
            .any(|p| p.path == exe && p.what == "the server's executable")
    );
    // A grant may read the executable's directory, not write it.
    std::fs::write(tree.path("x/tilrd"), "").unwrap();
    let mut dirs = tree.dirs.clone();
    dirs.write_protected.push(Protected {
        path: tree.path("x/tilrd"),
        what: "the server's executable".into(),
    });
    let refuse =
        |path: PathBuf, write: bool, dirs: &Dirs| refuse_exposing([(path.as_path(), write)], dirs);
    let err = refuse(tree.path("x"), true, &dirs).unwrap_err();
    assert!(
        err.contains("would expose the server's executable"),
        "{err}"
    );
    refuse(tree.path("x"), false, &dirs).unwrap();
    refuse(tree.path("x/tilrd"), false, &dirs).unwrap();
    // A configuration file given with --config elsewhere, a profiles directory of its own.
    dirs.protect_config_file(&tree.path("o/conf/server.toml"));
    dirs.protect_profiles_dir(&tree.path("r/profiles"));
    let err = refuse(tree.path("o"), false, &dirs).unwrap_err();
    assert!(
        err.contains("the configuration directory (--config)"),
        "{err}"
    );
    let err = refuse(tree.path("r/profiles/p.toml"), false, &dirs).unwrap_err();
    assert!(err.contains("the profiles directory"), "{err}");
    refuse(tree.path("o/elsewhere"), true, &dirs).unwrap();
}

#[test]
fn a_request_reaches_the_pane_process_in_a_private_file() {
    let tree = Tree::new("handover");
    let mut dirs = tree.dirs.clone();
    // A server started with --config elsewhere: the pane process learns it from the file.
    dirs.protect_config_file(&tree.path("o/conf/server.toml"));
    let handover = Handover {
        request: Request {
            read_only: vec!["/a".into()],
            read_write: vec!["/b".into()],
            executables: vec!["git".into(), "/c".into()],
            pass_env: vec!["LANG".into()],
            network: Network::Client,
            env: vec![("API_TOKEN".into(), "value-only-in-the-file".into())],
        },
        dirs,
    };
    let runtime = &tree.dirs.runtime;
    let args = handover.write("p1").unwrap();
    // The pane process's command line names the file; no name or value of [env] is on it.
    let line = args.join(std::ffi::OsStr::new(" "));
    let line = line.to_str().unwrap();
    assert!(
        !line.contains("value-only-in-the-file") && !line.contains("API_TOKEN"),
        "{line}"
    );
    let file = request_path(runtime, "p1");
    assert_eq!(args, [OsString::from("--sandbox"), file.clone().into()]);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(file.parent().unwrap()), 0o700);
    // In the runtime directory, which no grant covers, and not in the pane's TMPDIR.
    assert!(file.starts_with(runtime));
    assert!(!file.starts_with(pane_dir(runtime, "p1")));
    // The pane process takes those arguments, and the request whole from the file.
    #[derive(clap::Parser)]
    struct Pane {
        #[command(flatten)]
        args: crate::pane::Args,
    }
    let id = "00000000-0000-0000-0000-000000000001";
    let pane = <Pane as clap::Parser>::try_parse_from(
        [OsString::from("pane"), "--id".into(), id.into()]
            .into_iter()
            .chain(args),
    )
    .unwrap();
    assert_eq!(pane.args.sandbox.as_deref(), Some(file.as_path()));
    assert_eq!(Handover::read(&file).unwrap(), handover);
    // Handed over again (a pane id reused), the file is replaced.
    let again = Handover {
        request: Request::default(),
        dirs: tree.dirs.clone(),
    };
    again.write("p1").unwrap();
    assert_eq!(Handover::read(&file).unwrap(), again);
    // It goes when the pane ends, or at start when its pane does not run.
    remove_pane_dir(runtime, "p1");
    assert!(!file.exists());
    handover.write("p2").unwrap();
    remove_leftovers(runtime, |id| id == "p3");
    assert!(!request_path(runtime, "p2").exists());
    std::fs::write(&file, "{\"request\":{\"read_onlyy\":[]}}").unwrap();
    assert!(Handover::read(&file).unwrap_err().starts_with("sandbox: "));
    assert!(handover.write("../x").is_err());
}

#[test]
fn a_grant_of_the_users_terminals_is_warned_about() {
    let warn = |read_only: &[&str], read_write: &[&str]| {
        let request = Request {
            read_only: read_only.iter().map(PathBuf::from).collect(),
            read_write: read_write.iter().map(PathBuf::from).collect(),
            ..Request::default()
        };
        terminals_warning(&request, "p1")
    };
    assert_eq!(
        warn(&[], &["/dev/pts"]).as_deref(),
        Some(
            "sandbox: the grant /dev/pts covers /dev/pts: pane p1 can open every terminal of \
             the user, read what is typed there and write to it"
        )
    );
    for (read_only, read_write) in [
        (&["/dev"][..], &[][..]),
        (&["/dev/pts/3"], &[]),
        (&[], &["/"]),
    ] {
        assert!(
            warn(read_only, read_write).is_some(),
            "{read_only:?} {read_write:?}"
        );
    }
    assert_eq!(warn(&["/usr/share"], &["/dev/null"]), None);
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
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
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

    /// The real file of `/bin/sh`: a program of its own, which a renamed copy still is
    /// (a multicall `/bin/echo`, as Rust coreutils makes it, is not).
    fn shell_program() -> PathBuf {
        std::fs::canonicalize("/bin/sh").unwrap()
    }

    /// A copy of [`shell_program`] at `to`; `<to> -c 'echo X'` prints `X`.
    fn copy_program(to: &Path) {
        std::fs::copy(shell_program(), to).unwrap();
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
        let script = format!(
            "{o}/tool -c 'echo A'; echo rc=$?; {x}/granted -c 'echo B'; byname -c 'echo C'; ls {x} || echo nolist"
        );
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
             cp {sh} {w}/e && {w}/e -c 'echo hi'",
            sh = shell_program().display()
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
        let outside = Killed(
            std::process::Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .unwrap(),
        );
        let script = format!("kill -0 {} && echo reached || echo refused", outside.0.id());
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
    }

    /// A process of a test, killed and reaped however the test ends.
    struct Killed(std::process::Child);

    impl Drop for Killed {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A datagram socket made with `socketpair` is unbound and could `sendto` any named
    /// or abstract Unix socket where Landlock does not govern that (before ABI 6 for
    /// abstract ones): only connected streams and sequenced packets are made.
    #[tokio::test]
    async fn a_unix_datagram_socket_cannot_be_made_with_socketpair() {
        use std::os::linux::net::SocketAddrExt;
        let tree = Tree::new("dgram");
        let named = tree.path("o/d.sock");
        let receiver = std::os::unix::net::UnixDatagram::bind(&named).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let name = format!("tilrd-sandbox-{}", std::process::id());
        let abstract_name =
            std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let abstract_receiver =
            std::os::unix::net::UnixDatagram::bind_addr(&abstract_name).unwrap();
        abstract_receiver.set_nonblocking(true).unwrap();
        let named = named.display().to_string();
        let code = format!(
            "{TRY}\
             def dgram(kind):\n\
             \x20   a, b = socket.socketpair(socket.AF_UNIX, kind)\n\
             \x20   a.sendto(b'named', {named:?}); a.sendto(b'abstract', b'\\0{name}')\n\
             attempt('dgram', lambda: dgram(socket.SOCK_DGRAM))\n\
             attempt('dgram-cloexec', lambda: dgram(socket.SOCK_DGRAM | socket.SOCK_CLOEXEC))\n\
             attempt('stream', lambda: socket.socketpair())\n\
             def seqpacket():\n\
             \x20   a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)\n\
             \x20   a.sendto(b'x', {named:?}); assert b.recv(8) == b'x'\n\
             attempt('seqpacket', seqpacket)\n\
             def stream_sendto():\n\
             \x20   a, b = socket.socketpair(); a.sendto(b'x', {named:?})\n\
             attempt('stream-sendto', stream_sendto)\n"
        );
        // ABI 4 as well: no Landlock rule there for abstract sockets or named datagrams.
        for abi in [None, Some(4)] {
            ABI.set(abi);
            let out = python(&tree, &tree.request(Network::None), &code).await;
            ABI.set(None);
            assert!(
                out.contains("dgram EACCES\n") && out.contains("dgram-cloexec EACCES"),
                "{abi:?}: {out}"
            );
            assert!(
                out.contains("stream ok") && out.contains("seqpacket ok"),
                "{abi:?}: {out}"
            );
            // A connected pair sends only to its peer.
            assert!(out.contains("stream-sendto EISCONN"), "{abi:?}: {out}");
            let mut buf = [0u8; 16];
            assert!(receiver.recv(&mut buf).is_err(), "{abi:?}: {out}");
            assert!(abstract_receiver.recv(&mut buf).is_err(), "{abi:?}: {out}");
        }
    }

    /// The netlink a name lookup needs (routes and addresses) opens; another does not.
    #[tokio::test]
    async fn only_routing_netlink_opens_in_a_sandbox() {
        let tree = Tree::new("netlink");
        let code = format!(
            "{TRY}\
             raw = socket.SOCK_RAW\n\
             attempt('route', lambda: socket.socket(socket.AF_NETLINK, raw, 0))\n\
             attempt('sock_diag', lambda: socket.socket(socket.AF_NETLINK, raw, 4))\n\
             attempt('audit', lambda: socket.socket(socket.AF_NETLINK, raw, 9))\n\
             attempt('lookup', lambda: socket.getaddrinfo('localhost', 80, \
                 flags=socket.AI_ADDRCONFIG))\n"
        );
        let out = python(&tree, &tree.request(Network::Client), &code).await;
        assert!(
            out.contains("route ok") && out.contains("lookup ok"),
            "{out}"
        );
        assert!(
            out.contains("sock_diag EACCES") && out.contains("audit EACCES"),
            "{out}"
        );
    }

    /// A user namespace would give the pane capabilities of its own: `unshare` and
    /// `clone` with a namespace flag are refused, `clone3` does not exist, and forks,
    /// threads and `subprocess` (glibc falls back to `clone`) work as before.
    #[tokio::test]
    async fn no_namespace_can_be_made_in_a_sandbox() {
        let tree = Tree::new("namespaces");
        let code = format!(
            "import ctypes, os, subprocess, threading\n\
             libc = ctypes.CDLL(None, use_errno=True)\n\
             def call(label, nr, *args):\n\
             \x20   ctypes.set_errno(0); rc = libc.syscall(nr, *args)\n\
             \x20   if rc == 0 and label == 'clone': os._exit(0)\n\
             \x20   print(label, rc, ctypes.get_errno(), flush=True)\n\
             call('unshare', {unshare}, 0x10000000)\n\
             call('unshare-net', {unshare}, 0x40000000)\n\
             call('clone', {clone}, 0x10000000 | 17, 0, 0, 0, 0)\n\
             call('clone3', {clone3}, 0, 0)\n\
             pid = os.fork()\n\
             if pid == 0: os._exit(7)\n\
             print('fork', os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1]), flush=True)\n\
             t = threading.Thread(target=lambda: print('thread ok', flush=True)); t.start(); t.join()\n\
             print('subprocess', subprocess.run(['/bin/echo', 'ran'], capture_output=True).stdout, flush=True)\n",
            unshare = libc::SYS_unshare,
            clone = libc::SYS_clone,
            clone3 = libc::SYS_clone3,
        );
        let out = python(&tree, &tree.request(Network::None), &code).await;
        for label in ["unshare", "unshare-net", "clone"] {
            assert!(out.contains(&format!("{label} -1 1\n")), "{label}: {out}");
        }
        assert!(out.contains("clone3 -1 38\n"), "{out}");
        assert!(out.contains("fork 7") && out.contains("thread ok"), "{out}");
        assert!(out.contains("subprocess b'ran\\n'"), "{out}");
    }

    /// What a terminal program does not need of the kernel: io_uring (`EACCES`, as the
    /// socket rules), the keyrings, BPF, perf events, userfaultfd and `setns` (`EPERM`).
    #[tokio::test]
    async fn kernel_interfaces_a_pane_does_not_need_are_refused() {
        let tree = Tree::new("kernel");
        let calls = [
            ("io_uring_setup", libc::SYS_io_uring_setup, "1, params"),
            ("keyctl", libc::SYS_keyctl, "0, -4, 0"),
            ("add_key", libc::SYS_add_key, "b'user', b'k', b'v', 1, -4"),
            (
                "request_key",
                libc::SYS_request_key,
                "b'user', b'k', None, 0",
            ),
            ("bpf", libc::SYS_bpf, "0, params, 72"),
            (
                "perf_event_open",
                libc::SYS_perf_event_open,
                "params, 0, -1, -1, 0",
            ),
            ("userfaultfd", libc::SYS_userfaultfd, "0"),
            ("setns", libc::SYS_setns, "0, 0"),
        ];
        let mut code = "import ctypes\n\
             libc = ctypes.CDLL(None, use_errno=True)\n\
             params = ctypes.create_string_buffer(256)\n"
            .to_owned();
        for (label, nr, args) in calls {
            code.push_str(&format!(
                "ctypes.set_errno(0); rc = libc.syscall({nr}, {args}); \
                 print('{label}', rc, ctypes.get_errno(), flush=True)\n"
            ));
        }
        let out = python(&tree, &tree.request(Network::None), &code).await;
        assert!(out.contains("io_uring_setup -1 13\n"), "{out}");
        for (label, _, _) in &calls[1..] {
            assert!(out.contains(&format!("{label} -1 1\n")), "{label}: {out}");
        }
    }

    /// A descriptor the pane process holds without close-on-exec does not reach the
    /// program: the child marks every one above 2 before the exec.
    #[tokio::test]
    async fn no_descriptor_of_the_pane_process_reaches_the_program() {
        let tree = Tree::new("fds");
        let file = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: plain fcntl calls on a descriptor this test owns.
        let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD, 100) };
        assert!(fd >= 100);
        // SAFETY: `fd` was just made and is closed at the end of the test.
        let held = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: as above.
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, 0) }, 0);
        let script = format!("[ -e /proc/self/fd/{fd} ] && echo held || echo closed");
        let (_, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert_eq!(out, "closed\n");
        drop(held);
    }

    /// The check of a grant is made on what is opened for the rule: a path that became a
    /// link to a server directory after the request was looked at is refused.
    #[test]
    fn a_grant_that_became_a_link_to_a_server_directory_is_refused() {
        let tree = Tree::new("swap");
        std::os::unix::fs::symlink(tree.path("state"), tree.path("w/late")).unwrap();
        std::fs::write(tree.path("x/tilrd"), "").unwrap();
        std::os::unix::fs::symlink(tree.path("x"), tree.path("w/bin")).unwrap();
        let mut dirs = tree.dirs.clone();
        dirs.write_protected.push(Protected {
            path: tree.path("x/tilrd"),
            what: "the server's executable".into(),
        });
        let features = features().expect("this host offers the sandbox");
        let prepare = |path: &str, access| {
            let grants = [Grant {
                path: tree.path(path),
                access,
                checked: true,
            }];
            Current::prepare(&grants, Network::None, features, &dirs).err()
        };
        let err = prepare("w/late", Access::ReadOnly).expect("the grant is refused");
        assert!(err.contains("would expose the state directory"), "{err}");
        let err = prepare("w/bin", Access::ReadWrite).expect("the grant is refused");
        assert!(
            err.contains("would expose the server's executable"),
            "{err}"
        );
        assert_eq!(prepare("w/bin", Access::ReadOnly), None);
        assert_eq!(prepare("w", Access::ReadWrite), None);
    }

    /// The way around the default for a profile that wants tmux or `script` inside, and
    /// its price: with `/dev/pts` granted, a terminal made inside works, and so does
    /// another terminal of the user.
    #[tokio::test]
    async fn a_profile_granting_dev_pts_can_make_terminals() {
        let tree = Tree::new("pts");
        let (other_master, other_slave) = crate::pty::open_pty(80, 24).unwrap();
        let other = std::fs::read_link(format!("/proc/self/fd/{}", other_slave.as_raw_fd()))
            .unwrap()
            .display()
            .to_string();
        let mut request = tree.request(Network::None);
        request.read_write.push("/dev/pts".into());
        let code = format!(
            "import os\n\
             m, s = os.openpty(); os.write(s, b'inner\\n'); print('openpty', os.read(m, 64), flush=True)\n\
             f = os.open({other:?}, os.O_RDWR | os.O_NOCTTY); os.write(f, b'INTO-THE-OTHER\\n')\n\
             print('other ok', flush=True)\n"
        );
        let (code, out) = start(&tree, &request, &["python3", "-I", "-c", &code])
            .unwrap()
            .finish()
            .await;
        assert_eq!(code, Some(0), "{out}");
        assert!(out.contains("openpty b'inner\\r\\n'"), "{out}");
        assert!(out.contains("other ok"), "{out}");
        // The other terminal got what the pane wrote.
        let mut poll = libc::pollfd {
            fd: other_master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(LIMIT.as_millis()).unwrap();
        // SAFETY: one valid pollfd.
        assert_eq!(unsafe { libc::poll(&raw mut poll, 1, ms) }, 1);
        let mut buf = [0u8; 64];
        // SAFETY: `buf` is valid for writes of its length.
        let n = unsafe { libc::read(other_master.as_raw_fd(), buf.as_mut_ptr().cast(), 64) };
        let got = String::from_utf8_lossy(&buf[..usize::try_from(n).unwrap()]).to_string();
        assert!(got.contains("INTO-THE-OTHER"), "{got:?}");
    }

    /// A limit: Landlock has no right for metadata, so a sandboxed program changes the
    /// mode, times and extended attributes of any file of the user, the server's token
    /// included, though it can read or write the contents of neither.
    #[tokio::test]
    async fn metadata_outside_the_grants_can_be_changed() {
        let tree = Tree::new("metadata");
        for file in ["o/f", "state/token"] {
            std::fs::write(tree.path(file), "x").unwrap();
            std::fs::set_permissions(tree.path(file), std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let (f, token) = (tree.text("o/f"), tree.text("state/token"));
        let script = format!(
            "chmod 0644 {token} && echo chmod-token; touch -m -d 2000-01-01 {f} && echo touch; \
             cat {token} || echo no-read"
        );
        let (_, out) = run(&tree, &tree.request(Network::None), &script).await;
        assert!(
            out.contains("chmod-token") && out.contains("touch"),
            "{out}"
        );
        assert!(out.contains("no-read"), "{out}");
        let mode = std::fs::metadata(tree.path("state/token"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
        let mtime = std::fs::metadata(tree.path("o/f"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(mtime < std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000));
    }

    /// A limit: a directory that is not granted cannot be listed, but an inotify watch on
    /// it learns the names made there.
    #[tokio::test]
    async fn names_in_a_directory_not_granted_can_be_watched() {
        let tree = Tree::new("inotify");
        let o = tree.text("o");
        let code = format!(
            "import ctypes, os\n\
             libc = ctypes.CDLL(None, use_errno=True)\n\
             try:\n\
             \x20   os.listdir({o:?}); print('listed', flush=True)\n\
             except OSError as err:\n\
             \x20   print('list', err.errno, flush=True)\n\
             fd = libc.inotify_init()\n\
             print('watch', libc.inotify_add_watch(fd, {o:?}.encode(), 0x100), flush=True)\n\
             event = os.read(fd, 4096)\n\
             print('event', event[16:].rstrip(b'\\0'), flush=True)\n"
        );
        let mut term = start(
            &tree,
            &tree.request(Network::None),
            &["python3", "-I", "-c", &code],
        )
        .unwrap();
        term.expect("list 13\n");
        term.expect("watch 1\n");
        std::fs::write(tree.path("o/a-name"), "").unwrap();
        term.expect("event b'a-name'");
        let (code, out) = term.finish().await;
        assert_eq!(code, Some(0), "{out}");
    }

    /// A limit: what a sandboxed program can read it can run, here from memory
    /// (`memfd_create` and `execve`), as it could copy it into its `TMPDIR`.
    #[tokio::test]
    async fn a_readable_program_runs_from_memory_too() {
        let tree = Tree::new("memfd");
        copy_program(&tree.path("r/tool"));
        let code = format!(
            "import os\n\
             fd = os.memfd_create('tool')\n\
             os.write(fd, open({:?}, 'rb').read())\n\
             os.execve(fd, ['tool', '-c', 'echo ran-from-memory'], {{'PATH': '/usr/bin:/bin'}})\n",
            tree.text("r/tool")
        );
        let out = python(&tree, &tree.request(Network::None), &code).await;
        assert_eq!(out, "ran-from-memory\n");
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
            // os.write, not print: Ctrl+Z may stop the program inside the write of
            // STARTED, and the handler must not reenter a buffered stdout.
            "import os, signal\n\
             signal.signal(signal.SIGCONT, lambda *_: os.write(1, b'RESUMED\\n'))\n\
             os.write(1, b'STARTED\\n')\n\
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
