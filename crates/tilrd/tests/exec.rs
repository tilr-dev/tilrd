//! Use cases: Exec. A client runs a program on the server's machine without
//! a pane or a shell (`ExecStart`), gets its stdout and stderr apart
//! (`ExecOutput`) and how it ended (`ExecExit`), feeds its stdin
//! (`ExecInput`, acknowledged by `ExecInputAck`), signals or cancels it.

mod common;

use std::collections::HashMap;

use common::*;
use tilr_proto::{ExecCancel, ExecExit, ExecInput, ExecSignal, ExecStart, ExecStarted};

fn start_with_config(config: &str, args: &[&str]) -> Server {
    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/server.toml"), config).unwrap();
    Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), args)
}

fn argv(args: &[&str]) -> ExecStart {
    ExecStart {
        request_id: 1,
        argv: args.iter().map(|a| (*a).to_owned()).collect(),
        ..ExecStart::default()
    }
}

/// Start a program and return the answer.
async fn try_start(client: &mut Client, start: ExecStart) -> ExecStarted {
    let request_id = start.request_id;
    client.send(Msg::ExecStart(start)).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::ExecStarted(started) if started.request_id == request_id => {
                Some(started.clone())
            }
            _ => None,
        })
        .await
}

/// Start a program, which must start; its exec id and pid.
async fn start(client: &mut Client, start: ExecStart) -> (u64, u32) {
    let started = try_start(client, start).await;
    assert_eq!(started.error, "");
    assert_ne!(started.exec_id, 0);
    (started.exec_id, started.pid)
}

/// What a program wrote and how it ended.
#[derive(Debug, Default)]
struct Ran {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit: ExecExit,
}

impl Ran {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Read until the exec's `ExecExit`, which must come after all its output.
async fn finish(client: &mut Client, exec_id: u64) -> Ran {
    let mut ran = Ran::default();
    loop {
        match client.next().await.expect("connection closed") {
            server_message::Msg::ExecOutput(output) if output.exec_id == exec_id => {
                match output.stream {
                    1 => ran.stdout.extend_from_slice(&output.data),
                    2 => ran.stderr.extend_from_slice(&output.data),
                    other => panic!("stream {other}"),
                }
            }
            server_message::Msg::ExecExit(exit) if exit.exec_id == exec_id => {
                ran.exit = exit;
                return ran;
            }
            _ => {}
        }
    }
}

async fn run(client: &mut Client, exec: ExecStart) -> Ran {
    let (exec_id, _) = start(client, exec).await;
    finish(client, exec_id).await
}

/// A file that a program writes its background child's pid to.
fn pid_from(file: &Path) -> u32 {
    std::fs::read_to_string(file)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn exec_is_offered_unless_turned_off() {
    let server = Server::start(&[]);
    let mut client = Client::connect_silent(&server).await;
    let hello = client.hello().await;
    assert!(
        hello.features.contains(&"exec".to_owned()),
        "{:?}",
        hello.features
    );

    let flag = Server::start(&["--no-exec"]);
    let config = start_with_config("exec = false\n", &[]);
    for server in [&flag, &config] {
        let mut client = Client::connect_silent(server).await;
        let hello = client.hello().await;
        assert!(
            !hello.features.contains(&"exec".to_owned()),
            "{:?}",
            hello.features
        );
        // Refused with a reason; the connection goes on.
        client.send(Msg::ExecStart(argv(&["true"]))).await;
        client.ping(5).await;
        let refused = client
            .wait_for(|msg| match msg {
                server_message::Msg::ExecStarted(started) => Some(Some(started.clone())),
                server_message::Msg::Pong(_) => Some(None),
                _ => None,
            })
            .await
            .expect("ExecStarted before Pong");
        assert_eq!(
            (refused.request_id, refused.exec_id, refused.error.as_str()),
            (1, 0, "exec is turned off")
        );
        assert!(
            client
                .wait_for(|msg| match msg {
                    server_message::Msg::Pong(_) => Some(true),
                    _ => None,
                })
                .await
        );
    }
}

#[tokio::test]
async fn a_program_runs_without_a_shell_and_its_outputs_come_apart() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;

    let ran = run(
        &mut client,
        argv(&["sh", "-c", "echo out; echo err >&2; exit 3"]),
    )
    .await;
    assert_eq!(ran.stdout(), "out\n");
    assert_eq!(ran.stderr, b"err\n");
    assert_eq!((ran.exit.code, ran.exit.signal), (3, 0));
    assert!(!ran.exit.timed_out && !ran.exit.cancelled && ran.exit.reason.is_empty());

    // The arguments as they are: nothing expanded.
    let ran = run(
        &mut client,
        argv(&["printf", "%s|", "$HOME", "*", "`id`", "a b"]),
    )
    .await;
    assert_eq!(ran.stdout(), "$HOME|*|`id`|a b|");

    // The directory and the environment asked for; without a directory,
    // home.
    let dir = server.dir.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let ran = run(
        &mut client,
        ExecStart {
            cwd: dir.to_string_lossy().into(),
            env: vec!["TT_X=from exec".into()],
            ..argv(&["sh", "-c", "pwd -P; echo \"$TT_X\""])
        },
    )
    .await;
    assert_eq!(ran.stdout(), format!("{}\nfrom exec\n", dir.display()));
    let ran = run(&mut client, argv(&["sh", "-c", "pwd -P"])).await;
    // The tests' server has its scratch directory for home.
    let home = server.dir.canonicalize().unwrap();
    assert_eq!(ran.stdout(), format!("{}\n", home.display()));

    // Without stdin asked for, the program reads end of file.
    let ran = run(&mut client, argv(&["cat"])).await;
    assert_eq!((ran.stdout(), ran.exit.code), (String::new(), 0));

    // What cannot start is refused at once, and nothing else comes of it.
    for (exec, error) in [
        (argv(&["no-such-program-tt"]), "no-such-program-tt"),
        (argv(&[]), "no program"),
        (
            ExecStart {
                cwd: "/no/such/dir".into(),
                ..argv(&["true"])
            },
            "no such directory",
        ),
        (
            ExecStart {
                env: vec!["NOEQUALS".into()],
                ..argv(&["true"])
            },
            "NAME=value",
        ),
    ] {
        let started = try_start(&mut client, exec).await;
        assert_eq!(started.exec_id, 0);
        assert!(started.error.contains(error), "{}", started.error);
    }
    server.wait_logged("exec refused").await;
    server.wait_logged("exec started").await;
    server.wait_logged("exec ended").await;
}

#[tokio::test]
async fn the_signal_that_killed_a_program_is_reported() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let ran = run(&mut client, argv(&["sh", "-c", "kill -TERM $$"])).await;
    assert_eq!((ran.exit.code, ran.exit.signal), (0, 15));

    let (exec_id, _) = start(&mut client, argv(&["sleep", "30"])).await;
    client
        .send(Msg::ExecSignal(ExecSignal { exec_id, signal: 2 }))
        .await;
    let ran = finish(&mut client, exec_id).await;
    assert_eq!(ran.exit.signal, 2);
    assert!(!ran.exit.cancelled);
}

#[tokio::test]
async fn a_time_limit_kills_the_program_and_its_children() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("child-pid");
    let script = format!("sleep 30 & echo $! > '{}'; sleep 31; wait", file.display());
    let began = std::time::Instant::now();
    let (exec_id, pid) = start(
        &mut client,
        ExecStart {
            timeout_ms: 500,
            ..argv(&["sh", "-c", &script])
        },
    )
    .await;
    let ran = finish(&mut client, exec_id).await;
    assert!(ran.exit.timed_out);
    assert_eq!(ran.exit.signal, 9);
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "{:?}",
        began.elapsed()
    );
    let child = pid_from(&file);
    eventually("the program and its child are gone", || {
        !process_alive(pid) && !process_alive(child)
    })
    .await;
}

#[tokio::test]
async fn a_cancelled_program_ends_at_once_with_its_children() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("child-pid");
    let script = format!("sleep 40 & echo $! > '{}'; sleep 41; wait", file.display());
    let (exec_id, pid) = start(&mut client, argv(&["sh", "-c", &script])).await;
    eventually("the child runs", || {
        file.exists() && std::fs::metadata(&file).unwrap().len() > 0
    })
    .await;
    let child = pid_from(&file);
    assert!(process_alive(child));
    client.send(Msg::ExecCancel(ExecCancel { exec_id })).await;
    let ran = finish(&mut client, exec_id).await;
    assert!(ran.exit.cancelled && !ran.exit.timed_out);
    assert_eq!(ran.exit.signal, 9);
    eventually("the program and its child are gone", || {
        !process_alive(pid) && !process_alive(child)
    })
    .await;
}

#[tokio::test]
async fn what_a_program_leaves_running_in_its_group_goes_when_it_exits() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("child-pid");
    // The child keeps stdout open: the exec ends anyway, with the program.
    let script = format!("sleep 50 & echo $! > '{}'; echo bye", file.display());
    let began = std::time::Instant::now();
    let ran = run(&mut client, argv(&["sh", "-c", &script])).await;
    assert_eq!((ran.stdout(), ran.exit.code), ("bye\n".to_owned(), 0));
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    let child = pid_from(&file);
    eventually("the child is gone", || !process_alive(child)).await;
}

#[tokio::test]
async fn stdin_reaches_the_program_and_is_acknowledged() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let stdin = |args: &[&str]| ExecStart {
        stdin: true,
        ..argv(args)
    };

    // Closed at once: the program reads end of file.
    let (exec_id, _) = start(&mut client, stdin(&["wc", "-c"])).await;
    client
        .send(Msg::ExecInput(ExecInput {
            exec_id,
            data: Bytes::new(),
            eof: true,
        }))
        .await;
    assert_eq!(finish(&mut client, exec_id).await.stdout().trim(), "0");

    // 8 MiB in pieces, within the window, as the acknowledgements allow.
    let data = image(8 << 20);
    let (exec_id, _) = start(&mut client, stdin(&["wc", "-c"])).await;
    let (mut sent, mut taken) = (0u64, 0u64);
    for piece in data.chunks(64 * 1024) {
        while sent + piece.len() as u64 > taken + (4 << 20) {
            taken = client
                .wait_for(|msg| match msg {
                    server_message::Msg::ExecInputAck(ack) if ack.exec_id == exec_id => {
                        Some(ack.taken)
                    }
                    _ => None,
                })
                .await;
        }
        client
            .send(Msg::ExecInput(ExecInput {
                exec_id,
                data: Bytes::copy_from_slice(piece),
                eof: false,
            }))
            .await;
        sent += piece.len() as u64;
    }
    client
        .send(Msg::ExecInput(ExecInput {
            exec_id,
            data: Bytes::new(),
            eof: true,
        }))
        .await;
    let ran = finish(&mut client, exec_id).await;
    assert_eq!(ran.stdout().trim(), (8 << 20).to_string());

    // A program that does not read: more than the window ahead is an
    // overrun, and the exec is killed.
    let (exec_id, pid) = start(&mut client, stdin(&["sleep", "30"])).await;
    for _ in 0..5 {
        client
            .send(Msg::ExecInput(ExecInput {
                exec_id,
                data: Bytes::from(vec![0; 1 << 20]),
                eof: false,
            }))
            .await;
    }
    let ran = finish(&mut client, exec_id).await;
    assert_eq!(
        (ran.exit.reason.as_str(), ran.exit.signal),
        ("input overrun", 9)
    );
    eventually("the program is gone", || !process_alive(pid)).await;
}

#[tokio::test]
async fn a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let data = image(32 << 20);
    let file = server.dir.join("data");
    std::fs::write(&file, &data).unwrap();
    let done = server.dir.join("done");
    let script = format!(
        "cat '{}'; echo tail >&2; touch '{}'",
        file.display(),
        done.display()
    );
    let (exec_id, _) = start(&mut client, argv(&["sh", "-c", &script])).await;
    // The client reads nothing for a while: the program waits for it. The
    // window rules out output buffered without bound: `cat` of a file
    // writes 32 MiB in milliseconds when nothing holds it up, so it would
    // be done well within a second.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !done.exists(),
        "the program wrote 32 MiB to a client reading none"
    );
    let ran = finish(&mut client, exec_id).await;
    assert!(ran.stdout == data, "stdout whole and in order");
    assert_eq!(ran.stderr, b"tail\n");
    assert_eq!(ran.exit.code, 0);
}

#[tokio::test]
async fn execs_run_side_by_side_with_the_panes() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let began = std::time::Instant::now();
    let mut execs = Vec::new();
    for n in 0..8 {
        let script = format!("sleep 1; echo run {n}");
        execs.push(
            start(
                &mut client,
                ExecStart {
                    request_id: n + 1,
                    ..argv(&["sh", "-c", &script])
                },
            )
            .await
            .0,
        );
    }
    // Their outputs and exits come interleaved: gather them all.
    let mut outputs: HashMap<u64, Vec<u8>> = HashMap::new();
    let mut exits = 0;
    while exits < execs.len() {
        match client.next().await.expect("connection closed") {
            server_message::Msg::ExecOutput(output) => {
                outputs
                    .entry(output.exec_id)
                    .or_default()
                    .extend_from_slice(&output.data);
            }
            server_message::Msg::ExecExit(_) => exits += 1,
            _ => {}
        }
    }
    for (n, exec_id) in execs.iter().enumerate() {
        assert_eq!(outputs[exec_id], format!("run {n}\n").as_bytes());
    }
    assert!(
        began.elapsed() < Duration::from_secs(4),
        "{:?}",
        began.elapsed()
    );
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_client_that_goes_takes_its_programs_with_it() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("child-pid");
    let script = format!("sleep 60 & echo $! > '{}'; wait", file.display());
    let (_, pid) = start(&mut client, argv(&["sh", "-c", &script])).await;
    eventually("the child runs", || {
        file.exists() && std::fs::metadata(&file).unwrap().len() > 0
    })
    .await;
    let child = pid_from(&file);
    drop(client);
    eventually("the program and its child are gone", || {
        !process_alive(pid) && !process_alive(child)
    })
    .await;
    // The log says why.
    server.wait_logged("client gone").await;
}

#[tokio::test]
async fn a_reload_ends_the_programs_and_their_clients_hear_why() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let file = server.dir.join("child-pid");
    let script = format!(
        "sleep 60 & echo $! > '{}'; echo started; wait",
        file.display()
    );
    let (exec_id, pid) = start(&mut client, argv(&["sh", "-c", &script])).await;
    eventually("the child runs", || {
        file.exists() && std::fs::metadata(&file).unwrap().len() > 0
    })
    .await;
    let child = pid_from(&file);
    server.signal("-USR1");
    let ran = finish(&mut client, exec_id).await;
    assert_eq!(ran.stdout(), "started\n");
    assert_eq!((ran.exit.reason.as_str(), ran.exit.signal), ("reload", 9));
    eventually("the program and its child are gone", || {
        !process_alive(pid) && !process_alive(child)
    })
    .await;
    let mut client = reconnect(&server, client).await;
    let ran = run(&mut client, argv(&["echo", "again"])).await;
    assert_eq!(ran.stdout(), "again\n");
}

/// `sh -c` text that runs `script` in the background in a session of its
/// own, out of the exec's process group (`setsid`, which macOS has not as a
/// command), and goes on once it has left the group: it writes its pid to
/// `pid_file` then.
fn escaped(script: &str, pid_file: &Path) -> String {
    let file = pid_file.display();
    format!(
        "perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' sh -c 'echo $$ > {file}; {script}' & \
         while [ ! -s '{file}' ]; do sleep 0.01; done;"
    )
}

/// Kill a process the test left behind, if it is still there.
fn kill(pid_file: &Path) {
    if let Ok(pid) = std::fs::read_to_string(pid_file) {
        let _ = Command::new("kill")
            .args(["-9", pid.trim()])
            .stderr(Stdio::null())
            .status();
    }
}

#[tokio::test]
async fn a_process_outside_the_group_that_keeps_writing_does_not_hold_the_exec() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let writer = server.dir.join("writer-pid");
    let script = format!("{} echo bye", escaped("exec yes", &writer));
    let began = std::time::Instant::now();
    let ran = run(&mut client, argv(&["sh", "-c", &script])).await;
    let took = began.elapsed();
    kill(&writer);
    assert!(ran.stdout().contains("bye\n"), "{:?}", ran.stdout());
    assert_eq!(ran.exit.code, 0);
    assert!(took < Duration::from_secs(3), "{took:?}");
    // Its slot is free again: 64 execs run side by side.
    for n in 0..64 {
        let started = try_start(
            &mut client,
            ExecStart {
                request_id: 100 + n,
                ..argv(&["sleep", "30"])
            },
        )
        .await;
        assert_eq!(started.error, "", "exec {n}");
    }
}

#[tokio::test]
async fn a_cancel_during_the_drain_ends_the_exec_at_once() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let writer = server.dir.join("writer-pid");
    let script = format!(
        "{} echo bye",
        escaped("while :; do echo x; sleep 0.01; done", &writer)
    );
    let (exec_id, pid) = start(&mut client, argv(&["sh", "-c", &script])).await;
    // The shell has exited; the pipe is held by the writer outside its group.
    eventually("the shell exits", || !process_alive(pid)).await;
    client.send(Msg::ExecCancel(ExecCancel { exec_id })).await;
    let cancelled = std::time::Instant::now();
    let ran = finish(&mut client, exec_id).await;
    let took = cancelled.elapsed();
    kill(&writer);
    assert!(ran.exit.cancelled, "{:?}", ran.exit);
    assert!(took < Duration::from_secs(1), "{took:?}");
}

#[tokio::test]
async fn an_exec_given_up_at_a_reload_ends_with_its_exec_exit() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Output that does not compress, so that it fills the lane.
    let (exec_id, _) = start(&mut client, argv(&["cat", "/dev/urandom"])).await;
    // The client reads nothing: its bulk lane fills, and the exec's readers
    // wait for room. The second only sets the scene: /dev/urandom fills the
    // lane in milliseconds, and were it not full yet, the exec would still
    // be running at the reload and end the same way; nothing below passes
    // because of the window.
    tokio::time::sleep(Duration::from_secs(1)).await;
    server.signal("-USR1");
    // The reload gives up waiting for the exec after a second, and waits a
    // second more for the clients to read what they were sent.
    server.wait_logged("exec ended without its status").await;
    let mut last = None;
    while let Some(msg) = client.next().await {
        match msg {
            server_message::Msg::ExecOutput(output) if output.exec_id == exec_id => {
                last = Some(Err(output.data.len()));
            }
            server_message::Msg::ExecExit(exit) if exit.exec_id == exec_id => {
                last = Some(Ok(exit));
            }
            _ => {}
        }
    }
    match last {
        Some(Ok(exit)) => assert_eq!(exit.reason, "reload"),
        other => panic!("the exec's last message was not its ExecExit: {other:?}"),
    }
}

#[tokio::test]
async fn an_exec_does_not_get_the_service_managers_variables() {
    let dir = Server::scratch_dir();
    let (_notify, env) = service_manager_env(&dir);
    let env: Vec<(&str, &str)> = env.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    let server = Server::start_with_env(dir, &[], &env);
    let mut client = Client::connect(&server).await;

    let ran = run(&mut client, argv(&["env"])).await;
    let seen = ran.stdout();
    for (name, _) in &env {
        assert!(
            !seen
                .lines()
                .any(|line| line.starts_with(&format!("{name}="))),
            "{name} in the exec: {seen}"
        );
    }
    // The exec's own `env` still sets any of them.
    let ran = run(
        &mut client,
        ExecStart {
            env: vec!["NOTIFY_SOCKET=/x".into()],
            ..argv(&["env"])
        },
    )
    .await;
    assert!(ran.stdout().lines().any(|line| line == "NOTIFY_SOCKET=/x"));
}

async fn nothing_before_pong(client: &mut Client, nonce: u64) {
    client.ping(nonce).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Pong(pong) if pong.nonce == nonce => Some(()),
            server_message::Msg::Error(_)
            | server_message::Msg::ExecInputAck(_)
            | server_message::Msg::ExecExit(_) => panic!("unwanted input was not ignored: {msg:?}"),
            _ => None,
        })
        .await;
}

#[tokio::test]
async fn input_that_is_not_wanted_is_ignored() {
    let server = Server::start(&[]);
    let mut owner = Client::connect(&server).await;
    let mut stranger = Client::connect(&server).await;
    owner.hello().await;
    stranger.hello().await;
    let (fifo, mut release) = activity_fifo(&server, "activity-release");
    let captured = server.dir.join("captured-input");
    let script = format!(
        "cat > '{}'; read -r go < '{}'; printf done; cat '{}'",
        captured.display(),
        fifo.display(),
        captured.display()
    );
    for stdin in [false, true] {
        let (exec_id, _) = start(
            &mut owner,
            ExecStart {
                stdin,
                ..argv(&["/bin/sh", "-c", &script])
            },
        )
        .await;
        if stdin {
            owner
                .send(Msg::ExecInput(ExecInput {
                    exec_id,
                    eof: true,
                    ..Default::default()
                }))
                .await;
        }
        owner
            .send(Msg::ExecInput(ExecInput {
                exec_id,
                data: Bytes::from_static(b"unwanted"),
                eof: false,
            }))
            .await;
        owner
            .send(Msg::ExecInput(ExecInput {
                exec_id,
                eof: true,
                ..Default::default()
            }))
            .await;
        nothing_before_pong(&mut owner, 101).await;
        release_activity_fifo(&mut release);
        let ran = finish(&mut owner, exec_id).await;
        assert_eq!(ran.stdout(), "done");
        assert!(ran.stderr.is_empty());
        assert_eq!(ran.exit.code, 0);
        assert_eq!(ran.exit.signal, 0);
    }
    let (exec_id, _) = start(
        &mut owner,
        ExecStart {
            stdin: true,
            ..argv(&["wc", "-c"])
        },
    )
    .await;
    stranger
        .send(Msg::ExecInput(ExecInput {
            exec_id,
            data: Bytes::from_static(b"stranger"),
            eof: true,
        }))
        .await;
    nothing_before_pong(&mut stranger, 102).await;
    owner
        .send(Msg::ExecInput(ExecInput {
            exec_id,
            data: Bytes::from_static(b"owner"),
            eof: true,
        }))
        .await;
    let ran = finish(&mut owner, exec_id).await;
    assert_eq!(ran.stdout().trim(), "5");
    assert_eq!(ran.exit.code, 0);
    assert_eq!(ran.exit.signal, 0);
}
