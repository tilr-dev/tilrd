//! Use cases: sandboxed panes (`sandbox-linux`): a pane of a profile with `[sandbox]`
//! runs restricted to its grants, over the bus. Real server, the owner on the Unix
//! socket; a pane's program writes what it saw into a FIFO the test reads to its end
//! (`Out`), and a terminal program is watched on `pane.output`, so every wait is on an
//! event. The sandbox is Linux's (Landlock ABI 4 or later and seccomp); elsewhere
//! only the feature check runs.

mod common;

use common::bus_services;
use tilr_bus::Bus;
use tilr_bus::server::{InfoReply, InfoRequest};

use common::bus_sandbox::Boxed;

/// The features `server.info` lists.
async fn features(bus: &Bus) -> Vec<String> {
    let info: InfoReply = bus_services::call(bus, "server.info", &InfoRequest {})
        .await
        .expect("server.info");
    info.features
}

#[tokio::test]
async fn sandbox_is_offered_where_it_can_be_enforced() {
    let boxed = Boxed::start("");
    let owner = boxed.owner().await;
    let offered = features(&owner).await.iter().any(|f| f == "sandbox");
    assert_eq!(offered, cfg!(target_os = "linux"));
    #[cfg(feature = "failpoints")]
    {
        // A host where the start-up check fails offers no sandbox.
        let off = Boxed::start("sandbox-unavailable=error");
        let bus = off.owner().await;
        assert!(!features(&bus).await.iter().any(|f| f == "sandbox"));
        #[cfg(target_os = "linux")]
        starts_without_signal_scoping_on_abi_4().await;
    }
}

#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_os = "linux")]
use common::bus::{BusOptions, BusServer};
#[cfg(target_os = "linux")]
use common::bus_panes::{Watching, call, raw};
#[cfg(target_os = "linux")]
use common::bus_profiles::{create, list_panes, secure_dir, write_in, write_profile};
#[cfg(target_os = "linux")]
use common::bus_sandbox::{Exit, leftovers, shows};
#[cfg(target_os = "linux")]
use common::bus_streams::within;
#[cfg(target_os = "linux")]
use tilr_bus::BusError;
#[cfg(target_os = "linux")]
use tilr_bus::pane::{CloseReply, CloseRequest, PaneActivity, PaneActivityKind};

/// Landlock ABI 4 (the failpoint `sandbox-abi-4`): offered, and a pane starts
/// with the warning that it can signal the user's other processes.
#[cfg(target_os = "linux")]
async fn starts_without_signal_scoping_on_abi_4() {
    let boxed = Boxed::start("sandbox-abi-4=error");
    let bus = boxed.owner().await;
    assert!(features(&bus).await.iter().any(|f| f == "sandbox"));
    boxed.profile("box", "", "");
    let (pane, out) = boxed.run(&bus, "box", "echo ran").await;
    assert_eq!(out, "ran\n");
    boxed
        .wait_logged(&format!(
            "sandbox: kernel lacks signal scoping (Landlock ABI 4 < 6); pane {}",
            pane.pane_id
        ))
        .await;
}

/// `pane.create` refused: its error, no pane listed.
#[cfg(target_os = "linux")]
async fn refused(bus: &Bus, profile: &str, command: &[&str]) -> BusError {
    let err = create(bus, profile, command, "").await.unwrap_err();
    assert!(list_panes(bus).await.is_empty());
    err
}

#[cfg(target_os = "linux")]
fn copy_program(to: &std::path::Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy("/bin/echo", to).unwrap();
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_profile_starts_a_sandboxed_pane() {
    let boxed = Boxed::start("");
    boxed.profile("box", "[env]\nAPI_TOKEN = \"tok-123\"", "");
    let bus = boxed.owner().await;
    let mut out = boxed.out("token");
    let command = format!(
        "printf %s \"$API_TOKEN\" > '{}'; exec cat",
        out.path.display()
    );
    let pane = create(&bus, "box", &["/bin/sh", "-c", &command], "")
        .await
        .unwrap();
    assert!(pane.sandboxed);
    assert_eq!(pane.profile, "box");
    // The profile's [env] reached the program.
    assert_eq!(out.end().await, "tok-123");
    let plain = create(&bus, "", &["/bin/sh"], "").await.unwrap();
    assert!(!plain.sandboxed);
    let listed = list_panes(&bus).await;
    let sandboxed = |id: &str| listed.iter().find(|p| p.pane_id == id).unwrap().sandboxed;
    assert!(sandboxed(&pane.pane_id) && !sandboxed(&plain.pane_id));
    // The pane process got the request in a private file (its command line names
    // it, then shows the pane's title), with no value of [env] on its command line.
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", pane.pid)).unwrap();
    let cmdline = String::from_utf8_lossy(&cmdline);
    assert!(!cmdline.contains("tok-123"), "{cmdline}");
    let [_, file] = leftovers(&boxed.dir.join("runtime"), &pane.pane_id);
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(String::from_utf8_lossy(&std::fs::read(&file).unwrap()).contains("tok-123"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_pane_writes_only_where_it_may() {
    let boxed = Boxed::start("");
    boxed.profile("box", "", "");
    std::fs::write(boxed.path("r/readable"), "ro-content\n").unwrap();
    std::fs::write(boxed.path("o/secret"), "SECRET\n").unwrap();
    let (w, r, o) = (boxed.text("w"), boxed.text("r"), boxed.text("o"));
    let bus = boxed.owner().await;
    let script = format!("touch {w}/a {r}/b {o}/c; cat {r}/readable {o}/secret; ls {o}; echo END");
    let (_, out) = boxed.run(&bus, "box", &script).await;
    assert!(boxed.path("w/a").exists(), "{out}");
    assert!(
        !boxed.path("r/b").exists() && !boxed.path("o/c").exists(),
        "{out}"
    );
    assert!(
        out.contains("ro-content") && !out.contains("SECRET"),
        "{out}"
    );
    assert_eq!(out.matches("Permission denied").count(), 4, "{out}");
    assert!(out.ends_with("END\n"), "{out}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_child_of_the_shell_is_restricted_too() {
    let boxed = Boxed::start("");
    boxed.profile("box", "", "");
    let o = boxed.text("o");
    let bus = boxed.owner().await;
    let script = format!("sh -c 'sh -c \"touch {o}/d\"'; echo rc=$?");
    let (_, out) = boxed.run(&bus, "box", &script).await;
    assert!(
        out.contains("Permission denied") && out.contains("rc=1"),
        "{out}"
    );
    assert!(!boxed.path("o/d").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn only_granted_programs_run() {
    let boxed = Boxed::start("");
    copy_program(&boxed.path("o/tool"));
    copy_program(&boxed.path("x/granted"));
    boxed.profile(
        "box",
        "",
        &format!("executables = [{:?}]", boxed.text("x/granted")),
    );
    let (o, x) = (boxed.text("o"), boxed.text("x"));
    let bus = boxed.owner().await;
    let script = format!("{o}/tool A; echo rc=$?; {x}/granted B");
    let (_, out) = boxed.run(&bus, "box", &script).await;
    assert!(
        out.contains("Permission denied") && out.contains("rc=126"),
        "{out}"
    );
    assert!(!out.contains("A\n") && out.ends_with("B\n"), "{out}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_script_and_a_binary_made_inside_run_inside() {
    let boxed = Boxed::start("");
    boxed.profile("box", "", "");
    let w = boxed.text("w");
    let bus = boxed.owner().await;
    let script = format!(
        "printf '#!/bin/sh\\necho ok\\n' > {w}/t.sh && chmod +x {w}/t.sh && {w}/t.sh && \
         cp /bin/echo {w}/e && {w}/e hi"
    );
    let (_, out) = boxed.run(&bus, "box", &script).await;
    assert_eq!(out, "ok\nhi\n");
}

/// Python that tries a network operation and prints its label with `ok` or the
/// errno's name.
#[cfg(target_os = "linux")]
const TRY: &str = "import errno, socket\n\
    def attempt(label, fn):\n\
    \x20   try:\n\
    \x20       fn()\n\
    \x20       print(label, 'ok', flush=True)\n\
    \x20   except OSError as err:\n\
    \x20       print(label, errno.errorcode.get(err.errno, err.errno), flush=True)\n";

/// `code` as a file in `r` the pane runs with `python3 -I`.
#[cfg(target_os = "linux")]
fn python(boxed: &Boxed, name: &str, code: &str) -> String {
    let file = boxed.path(&format!("r/{name}.py"));
    std::fs::write(&file, code).unwrap();
    format!("python3 -I {}", file.display())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn network_follows_the_profile() {
    let boxed = Boxed::start("");
    for network in ["none", "client", "server"] {
        boxed.profile(network, "", &format!("network = {network:?}"));
    }
    let bus = boxed.owner().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let tries = python(
        &boxed,
        "tries",
        &format!(
            "{TRY}\
             attempt('connect', lambda: socket.create_connection(('127.0.0.1', {port}), 5))\n\
             def listen():\n\
             \x20   s = socket.socket(); s.listen()\n\
             attempt('listen', listen)\n\
             attempt('sctp', lambda: socket.socket(socket.AF_INET, socket.SOCK_STREAM, 132))\n"
        ),
    );

    let (_, out) = boxed.run(&bus, "none", &tries).await;
    assert!(out.contains("connect EACCES"), "{out}");
    // The listener saw nothing: the program has ended, so a connection would wait.
    assert!(listener.accept().is_err());

    let (_, out) = boxed.run(&bus, "client", &tries).await;
    assert!(out.contains("connect ok"), "{out}");
    assert!(listener.accept().is_ok());
    assert!(
        out.contains("listen EACCES") && out.contains("sctp EACCES"),
        "{out}"
    );

    let serve = python(
        &boxed,
        "serve",
        "import socket\n\
         s = socket.socket(); s.bind(('127.0.0.1', 0)); s.listen()\n\
         print('port', s.getsockname()[1], flush=True)\n\
         c, _ = s.accept(); print('accepted', flush=True)\n",
    );
    let mut out = boxed.out("serve.out");
    let command = format!("{serve} > '{}' 2>&1", out.path.display());
    create(&bus, "server", &["/bin/sh", "-c", &command], "")
        .await
        .unwrap();
    let said = out.until("\n").await;
    let port: u16 = said
        .trim()
        .strip_prefix("port ")
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("{said}"));
    let _connection = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert_eq!(out.end().await, format!("port {port}\naccepted\n"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_command_from_the_request_runs_in_the_profiles_sandbox() {
    let boxed = Boxed::start("");
    let (w, o) = (boxed.text("w"), boxed.text("o"));
    boxed.profile(
        "box",
        &format!("command = [\"/bin/sh\", \"-c\", \"touch {w}/from-profile\"]"),
        "",
    );
    let bus = boxed.owner().await;
    let (pane, out) = boxed
        .run(&bus, "box", &format!("touch {w}/y {o}/x; echo END"))
        .await;
    assert!(pane.sandboxed);
    assert!(out.contains("Permission denied"), "{out}");
    assert!(boxed.path("w/y").exists() && !boxed.path("o/x").exists());
    assert!(!boxed.path("w/from-profile").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_pane_has_only_its_variables() {
    let boxed = Boxed::start_with_env(
        "",
        &[
            ("LANG", "C.UTF-8"),
            ("SECRET", "x"),
            ("SSH_AUTH_SOCK", "/x"),
        ],
    );
    boxed.profile(
        "box",
        "[env]\nGREETING = \"hi\"",
        "env = [\"LANG\", \"SSH_AUTH_SOCK\"]",
    );
    let bus = boxed.owner().await;
    let (pane, out) = boxed.run(&bus, "box", "env").await;
    let vars: Vec<(&str, &str)> = out
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let value = |name: &str| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    assert_eq!(value("LANG"), Some("C.UTF-8"), "{out}");
    assert_eq!(value("GREETING"), Some("hi"), "{out}");
    assert_eq!(value("TILR_PANE"), Some(pane.pane_id.as_str()), "{out}");
    assert!(value("TMPDIR").is_some(), "{out}");
    // What sh sets itself, and the list of the spec.
    let allowed = [
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "TERM",
        "COLORTERM",
        "TILR_PANE",
        "TMPDIR",
        "PATH",
        "LANG",
        "GREETING",
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ];
    for (name, _) in &vars {
        assert!(allowed.contains(name), "{name} in a sandboxed pane:\n{out}");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_pane_cannot_reach_the_server() {
    let boxed = Boxed::start("");
    boxed.profile("box", "", "network = \"server\"");
    let token = boxed.dir.join("state/token");
    assert!(token.exists(), "the server keeps a token");
    let bus = boxed.owner().await;
    let code = python(
        &boxed,
        "reach",
        &format!(
            "{TRY}\
             attempt('unix', lambda: socket.socket(socket.AF_UNIX).connect({:?}))\n\
             attempt('token', lambda: open({:?}).read())\n",
            boxed.socket().display().to_string(),
            token.display().to_string(),
        ),
    );
    let (_, out) = boxed.run(&bus, "box", &code).await;
    assert_eq!(out, "unix EACCES\ntoken EACCES\n");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_grant_over_the_state_directory_is_refused() {
    let boxed = Boxed::start("");
    let marker = boxed.path("w/marker");
    write_profile(
        &boxed,
        "home",
        &format!(
            "[sandbox]\nread_only = [{:?}]\n",
            boxed.dir.join("state").to_str().unwrap()
        ),
    );
    let bus = boxed.owner().await;
    let touch = format!("touch {}", marker.display());
    let err = refused(&bus, "home", &["/bin/sh", "-c", &touch]).await;
    assert_eq!(err.code, "bad_request");
    assert!(
        err.message.starts_with("create pane: sandbox: ")
            && err
                .message
                .contains(boxed.dir.join("state").to_str().unwrap()),
        "{}",
        err.message
    );
    assert!(!marker.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_grant_over_a_configuration_elsewhere_is_refused() {
    // `--config` names a file outside the default configuration directory, and
    // that file a profiles directory elsewhere again.
    let config = secure_dir();
    let profiles = secure_dir();
    let file = config.join("my.toml");
    std::fs::write(
        &file,
        format!("profiles_dir = {:?}\n", profiles.to_str().unwrap()),
    )
    .unwrap();
    let server = BusServer::start(BusOptions {
        args: vec!["--config".into(), file.to_str().unwrap().into()],
        ..BusOptions::default()
    });
    for (name, grant) in [("config", &config), ("profiles", &profiles)] {
        let text = format!(
            "command = [\"/bin/true\"]\n[sandbox]\nread_only = [{:?}]\n",
            grant.to_str().unwrap()
        );
        write_in(&profiles, name, &text, 0o600);
    }
    let bus = server.owner().await;
    let err = refused(&bus, "config", &[]).await;
    assert_eq!(err.code, "bad_request");
    assert!(
        err.message.starts_with("create pane: sandbox: ")
            && err
                .message
                .contains("the configuration directory (--config)"),
        "{}",
        err.message
    );
    let err = refused(&bus, "profiles", &[]).await;
    assert!(
        err.message.starts_with("create pane: sandbox: ")
            && err.message.contains("the profiles directory"),
        "{}",
        err.message
    );
}

#[cfg(target_os = "linux")]
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sandbox_that_cannot_be_applied_starts_nothing() {
    let boxed = Boxed::start("sandbox-apply=error");
    boxed.profile("box", "", "");
    let marker = boxed.path("w/marker");
    let bus = boxed.owner().await;
    let touch = format!("touch {}", marker.display());
    let err = refused(&bus, "box", &["/bin/sh", "-c", &touch]).await;
    assert_eq!(err.code, "bad_request");
    assert!(
        err.message.starts_with("create pane: sandbox: "),
        "{}",
        err.message
    );
    assert!(!marker.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_pager_and_an_editor_work_in_a_sandboxed_pane() {
    let boxed = Boxed::start("");
    // `less` keeps no history where it may not write.
    boxed.profile("box", "[env]\nLESSHISTFILE = \"-\"", "");
    let long: String = (1..=100).map(|n| format!("line-{n:03}\n")).collect();
    std::fs::write(boxed.path("r/long"), long).unwrap();
    let (long, file) = (boxed.text("r/long"), boxed.text("w/f"));
    let script = format!(
        "less {long}; echo LESS\"\"-DONE; read _; vi -i NONE {file}; echo VI\"\"-DONE; exec cat"
    );
    let bus = boxed.owner().await;
    let pane = create(&bus, "box", &["/bin/sh", "-c", &script], "")
        .await
        .unwrap();
    let id = pane.pane_id.as_str();
    let mut watch = Watching::open(&bus, id).await.unwrap();
    // A page of the file, the next one on Space, and the shell's turn on q.
    shows(&mut watch, "line-001").await;
    raw(&bus, id, " ").await.unwrap();
    shows(&mut watch, "line-040").await;
    raw(&bus, id, "q").await.unwrap();
    shows(&mut watch, "LESS-DONE").await;
    raw(&bus, id, "\r").await.unwrap();
    // vi has drawn the empty file: it reads keys now.
    shows(&mut watch, "[New").await;
    raw(&bus, id, "ihello\x1b:wq\r").await.unwrap();
    shows(&mut watch, "VI-DONE").await;
    assert_eq!(
        std::fs::read_to_string(boxed.path("w/f")).unwrap(),
        "hello\n"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn raw_mode_and_job_control_work_in_a_sandboxed_pane() {
    let boxed = Boxed::start("");
    boxed.profile(
        "bash",
        "command = [\"/bin/bash\", \"--norc\", \"--noprofile\", \"-i\"]\n\
         [env]\nPS1 = \"tilr$ \"",
        "",
    );
    std::fs::write(
        boxed.path("r/job.py"),
        // os.write, not print: Ctrl+Z may stop the program inside the write of
        // STARTED, and the handler must not reenter a buffered stdout.
        "import os, signal\n\
         signal.signal(signal.SIGCONT, lambda *_: os.write(1, b'RESUMED\\n'))\n\
         os.write(1, b'STARTED\\n')\n\
         while True:\n    signal.pause()\n",
    )
    .unwrap();
    let bus = boxed.owner().await;
    let pane = create(&bus, "bash", &[], "").await.unwrap();
    let id = pane.pane_id.as_str();
    let mut watch = Watching::open(&bus, id).await.unwrap();
    shows(&mut watch, "tilr$").await;
    let bad = [
        "process group",
        "no job control",
        "Inappropriate ioctl",
        "stty:",
    ];
    let clean = |screen: &str| {
        for bad in bad {
            assert!(!screen.contains(bad), "{bad}:\n{screen}");
        }
    };
    clean(&watch.screen.text());

    // Raw mode: one key is read without Enter. `RE""ADY` in the echoed line is
    // not `READY`.
    raw(
        &bus,
        id,
        "stty raw -echo; echo RE\"\"ADY; dd bs=1 count=1 2>/dev/null | od -c; stty sane; echo DO\"\"NE\r",
    )
    .await
    .unwrap();
    shows(&mut watch, "READY").await;
    raw(&bus, id, "x").await.unwrap();
    shows(&mut watch, "DONE").await;
    assert!(
        watch.screen.text().contains("0000000   x"),
        "{}",
        watch.screen.text()
    );
    clean(&watch.screen.text());

    // Job control: Ctrl+Z, jobs, fg, Ctrl+C.
    raw(&bus, id, "clear\r").await.unwrap();
    let job = boxed.text("r/job.py");
    raw(&bus, id, &format!("python3 -I {job}\r")).await.unwrap();
    shows(&mut watch, "STARTED").await;
    raw(&bus, id, "\x1a").await.unwrap();
    shows(&mut watch, "Stopped").await;
    raw(&bus, id, "jobs; echo JO\"\"BS\r").await.unwrap();
    shows(&mut watch, "JOBS").await;
    // The shell's prompt is back in the foreground: what finishes next is `fg`'s.
    let mut finished = bus.subscribe("pane.command_finished").await.unwrap();
    raw(&bus, id, "fg\r").await.unwrap();
    shows(&mut watch, "RESUMED").await;
    raw(&bus, id, "\x03").await.unwrap();
    shows(&mut watch, "KeyboardInterrupt").await;
    let event = within(finished.next())
        .await
        .expect("the subscription ended");
    assert_eq!(event.scope, format!("pane:{id}"));
    let activity: PaneActivity = prost::Message::decode(event.payload).unwrap();
    assert_eq!(activity.kind, PaneActivityKind::CommandFinished as i32);
    clean(&watch.screen.text());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_panes_leftovers_are_removed() {
    let mut boxed = Boxed::start("");
    boxed.profile("box", "", "");
    let runtime = boxed.dir.join("runtime");
    let bus = boxed.owner().await;
    // A pane that wrote into its TMPDIR and runs on.
    let start = async |bus: &Bus| {
        let mut out = boxed.out("made");
        let command = format!(
            "echo x > \"$TMPDIR/x\" && echo made > '{}'; exec cat",
            out.path.display()
        );
        let pane = create(bus, "box", &["/bin/sh", "-c", &command], "")
            .await
            .unwrap();
        assert_eq!(out.end().await, "made\n");
        std::fs::remove_file(&out.path).unwrap();
        let [dir, file] = leftovers(&runtime, &pane.pane_id);
        assert!(dir.join("x").exists() && file.exists());
        pane
    };

    // Closed: its directory and its request go with its pane process.
    let pane = start(&bus).await;
    let ended = Exit::watch(pane.pid);
    let _: CloseReply = call(
        &bus,
        "pane.close",
        &CloseRequest {
            pane_id: pane.pane_id.clone(),
        },
    )
    .await
    .unwrap();
    ended.ended().await;
    for left in leftovers(&runtime, &pane.pane_id) {
        assert!(!left.exists(), "{} is left", left.display());
    }

    // Killed with its server: what it left goes when the server starts again.
    let pane = start(&bus).await;
    let ended = Exit::watch(pane.pid);
    // SAFETY: kill takes a pid and a signal.
    assert_eq!(unsafe { libc::kill(pane.pid as i32, libc::SIGKILL) }, 0);
    ended.ended().await;
    drop(bus);
    let server = &mut boxed.server.server.server;
    server.signal("-KILL");
    server.child.wait().unwrap();
    for left in leftovers(&runtime, &pane.pane_id) {
        assert!(left.exists(), "{} is gone already", left.display());
    }
    let config = boxed.server.config.to_str().unwrap().to_owned();
    let again = common::Server::start_full(
        boxed.dir.clone(),
        std::path::Path::new(env!("CARGO_BIN_EXE_tilrd")),
        &[],
        "",
        &[("TILR_CONFIG_DIR", &config)],
    );
    again
        .wait_logged(&format!(
            "sandbox: removing the leftovers of pane {}",
            pane.pane_id
        ))
        .await;
    for left in leftovers(&runtime, &pane.pane_id) {
        assert!(!left.exists(), "{} is left", left.display());
    }
}
