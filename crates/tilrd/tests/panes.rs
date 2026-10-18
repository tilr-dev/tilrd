//! Use cases: connecting, and running a shell in a pane.

mod common;

use common::*;

#[tokio::test]
async fn hello_announces_the_clipboard_and_its_limit() {
    let server = Server::start(&["--clipboard-max", "3MB"]);
    let mut client = Client::connect(&server).await;
    let ack = client.hello().await;
    assert_eq!(ack.protocol_version, PROTOCOL_VERSION);
    assert_eq!(
        ack.features,
        ["clipboard", "files", "host-events", "agent", "exec"]
    );
    assert_eq!(ack.clipboard_max_bytes, 3 << 20);
}

#[tokio::test]
async fn a_pane_runs_commands_and_reports_how_its_shell_exited() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    client.hello().await;
    assert!(client.list_panes().await.is_empty());

    let pane_id = client.open_pane().await;
    client
        .type_line(pane_id, "echo \"sum is $((6 * 7))\"")
        .await;
    client.wait_screen("sum is 42").await;

    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, pane_id.to_string());
    assert_eq!(panes[0].viewers, 1);
    assert_eq!((panes[0].cols, panes[0].rows), (100, 30));

    client.type_line(pane_id, "exit 3").await;
    let exit_code = client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneExited(exited) if exited.pane_id == pane_id.to_string() => {
                Some(exited.exit_code)
            }
            _ => None,
        })
        .await;
    assert_eq!(exit_code, 3);
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn a_pane_without_a_command_runs_the_users_login_shell() {
    let server = Server::start(&[]);
    // Startup files of the scratch home, so that zsh does not offer its
    // new-user setup.
    for name in [".zshrc", ".zprofile"] {
        std::fs::write(server.dir.join(name), "").unwrap();
    }
    let mut client = Client::connect(&server).await;
    client.hello().await;
    client
        .send(Msg::CreatePane(CreatePane {
            cols: 100,
            rows: 30,
            command: Vec::new(),
            cwd: String::new(),
            request_id: 6,
        }))
        .await;
    let pane_id: PaneId = client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneCreated(created) if created.request_id == 6 => {
                Some(created.pane.as_ref().unwrap().pane_id.parse().unwrap())
            }
            server_message::Msg::Error(error) => panic!("create pane: {}", error.message),
            _ => None,
        })
        .await;
    client.subscribe(pane_id).await;

    // A login shell: started as `-zsh`, `-bash`, `-sh`, and told which shell
    // it is and whose.
    let check = "case \"$0\" in \"-${SHELL##*/}\") true;; *) false;; esac";
    assert_eq!(
        client.run(pane_id, check).await,
        0,
        "{}",
        client.screen.text()
    );
    let home = server.dir.display();
    let check = format!(
        "test -x \"$SHELL\" && test \"$HOME\" = '{home}' && test -n \"$USER\" && test \"$LOGNAME\" = \"$USER\""
    );
    assert_eq!(
        client.run(pane_id, &check).await,
        0,
        "{}",
        client.screen.text()
    );
    // The server's tools are still on its PATH after the login profile,
    // unless the system's profile sets PATH from scratch (Debian's
    // /etc/profile does): then TILR_SHIMS is what brings them back.
    let check = if system_profile_keeps_path() {
        "test -n \"$TILR_SHIMS\" && case \":$PATH:\" in *\":$TILR_SHIMS:\"*) true;; *) false;; esac"
    } else {
        // Here the test puts TILR_SHIMS on PATH itself: this only checks that
        // the login shell got TILR_SHIMS and the shims are there, not that a
        // user's profile brings them back onto PATH.
        "test -n \"$TILR_SHIMS\" && PATH=\"$TILR_SHIMS:$PATH\" && test \"$(command -v xclip)\" = \"$TILR_SHIMS/xclip\""
    };
    assert_eq!(
        client.run(pane_id, check).await,
        0,
        "{}",
        client.screen.text()
    );
}

/// Whether the system-wide login profile keeps the PATH a login shell is
/// started with, rather than setting one of its own.
fn system_profile_keeps_path() -> bool {
    let marker = "/tilr-test-path-marker";
    let output = Command::new("/bin/sh")
        .args(["-c", ". /etc/profile >/dev/null 2>&1; printf %s \"$PATH\""])
        .env("PATH", format!("{marker}:/usr/bin:/bin"))
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split(':')
        .any(|dir| dir == marker)
}

#[tokio::test]
async fn a_pane_starts_in_the_home_directory_unless_given_one_that_exists() {
    let server = Server::start(&[]);
    let home = server.dir.display().to_string();
    let given = server.dir.join("given");
    std::fs::create_dir(&given).unwrap();
    std::fs::write(server.dir.join("in-home"), "").unwrap();
    std::fs::write(given.join("in-given"), "").unwrap();
    let mut client = Client::connect(&server).await;

    // No directory, and one from another machine: the home directory.
    for cwd in ["", "/no/such/directory"] {
        let pane_id = client.open_pane_with(100, 30, cwd).await;
        assert_eq!(client.run(pane_id, "test -f in-home").await, 0);
        let panes = client.list_panes().await;
        assert_eq!(panes.last().unwrap().cwd, home);
    }

    let given = given.display().to_string();
    let pane_id = client.open_pane_with(100, 30, &given).await;
    assert_eq!(client.run(pane_id, "test -f in-given").await, 0);
    assert_eq!(client.list_panes().await.last().unwrap().cwd, given);
}

#[tokio::test]
async fn closing_a_pane_ends_its_shell() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .send(Msg::ClosePane(ClosePane {
            pane_id: pane_id.to_string(),
        }))
        .await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneExited(exited) if exited.pane_id == pane_id.to_string() => {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn a_message_type_the_server_does_not_know_is_ignored() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Flag byte, then field 99 of ClientMessage holding an empty message.
    let unknown = Bytes::from_static(&[0, 0x9a, 0x06, 0x00]);
    client.ws.send(Message::Binary(unknown)).await.unwrap();
    client
        .send(Msg::ListPanes(ListPanes { request_id: 1 }))
        .await;
    // The very next message answers the request: no error came before it.
    assert!(matches!(
        client.next().await,
        Some(server_message::Msg::PaneList(list)) if list.request_id == 1
    ));
}

#[tokio::test]
async fn a_browser_page_connects_only_from_an_allowed_origin() {
    let server = Server::start(&["--allow-origin", "http://app.test"]);
    // A page from anywhere else must not get a shell on this machine.
    let refused = Client::connect_from(&server, "http://evil.test").await;
    assert!(refused.is_err_and(|err| err.contains("403")));

    let mut page = Client::connect_from(&server, "http://app.test")
        .await
        .unwrap();
    assert_eq!(page.hello().await.protocol_version, PROTOCOL_VERSION);
    // Programs that are not browsers send no origin and are let in.
    let mut program = Client::connect(&server).await;
    assert_eq!(program.hello().await.protocol_version, PROTOCOL_VERSION);
}

#[tokio::test]
async fn the_configuration_file_names_the_allowed_origins() {
    // ~/.config/tilr/server.toml (here: the scratch directory's) sets the
    // list; --allow-origin on the command line replaces it.
    let exe = Path::new(env!("CARGO_BIN_EXE_tilrd"));
    let configured = |text: &str| {
        let dir = Server::scratch_dir();
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config/server.toml"), text).unwrap();
        dir
    };
    let file = "allow_origins = [\"tilr://my-notes\", \"http://app.test\"]\n";

    let server = Server::start_in(configured(file), exe, &[]);
    assert!(
        Client::connect_from(&server, "tilr://app")
            .await
            .is_err_and(|err| err.contains("403"))
    );
    let mut page = Client::connect_from(&server, "tilr://my-notes")
        .await
        .unwrap();
    assert_eq!(page.hello().await.protocol_version, PROTOCOL_VERSION);
    assert!(server.logged("configuration read"));

    let server = Server::start_in(
        configured(file),
        exe,
        &["--allow-origin", "http://cli.test"],
    );
    assert!(
        Client::connect_from(&server, "tilr://my-notes")
            .await
            .is_err_and(|err| err.contains("403"))
    );
    assert!(
        Client::connect_from(&server, "http://cli.test")
            .await
            .is_ok()
    );

    // A file that does not parse (or names a key the server does not know)
    // stops the server from starting, and says where the file is.
    let dir = configured("allow_origin = []\n");
    let output = std::process::Command::new(exe)
        .args(["--listen", "127.0.0.1:0"])
        .env("TILR_CONFIG_DIR", dir.join("config"))
        .env("TILR_STATE_DIR", dir.join("state"))
        .env("HOME", &dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("server.toml") && stderr.contains("allow_origin"),
        "{stderr}"
    );
}

#[tokio::test]
async fn a_malformed_packet_gets_an_error_and_the_connection_stays() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Flag byte 9 does not exist.
    client
        .ws
        .send(Message::Binary(Bytes::from_static(&[9, 1, 2])))
        .await
        .unwrap();
    assert!(client.error().await.contains("bad packet"));
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn asking_for_a_pane_that_does_not_exist_gets_an_error() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let nobody = PaneId::new_v4();
    client.subscribe(nobody).await;
    assert_eq!(client.error().await, format!("no such pane: {nobody}"));
    client.type_line(nobody, "echo").await;
    assert_eq!(client.error().await, format!("no such pane: {nobody}"));
    // A name that is no id at all is no pane either.
    client
        .send(Msg::Subscribe(Subscribe {
            pane_id: "1".into(),
        }))
        .await;
    assert_eq!(client.error().await, "no such pane: 1");
}

#[tokio::test]
async fn a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane_with(80, 24, "").await;
    let other = client.open_pane_with(100, 30, "").await;
    assert_ne!(pane_id, other);

    // What runs in a pane can ask which pane it is in.
    client.subscribe(pane_id).await;
    let check = format!("test \"$TILR_PANE\" = {pane_id}");
    assert_eq!(client.run(pane_id, &check).await, 0);

    // The pane process says which pane it is, and its size only where that
    // is not the usual one.
    let home = server.dir.display();
    let panes = client.list_panes().await;
    let args = |pane: &PaneInfo| {
        process_args(pane.pid)
            .split_once(" pane ")
            .unwrap()
            .1
            .to_owned()
    };
    assert_eq!(panes[0].pane_id, pane_id.to_string());
    assert_eq!(
        args(&panes[0]),
        format!("--id {pane_id} --cwd {home} -- /bin/sh")
    );
    assert_eq!(
        args(&panes[1]),
        format!("--id {other} --cols 100 --rows 30 --cwd {home} -- /bin/sh")
    );
}

#[tokio::test]
async fn ps_lists_the_panes_of_a_running_server() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = client.list_panes().await[0].pid;

    let output = Command::new(env!("CARGO_BIN_EXE_tilrd"))
        .args(["ps", "--url", &server.addr])
        // Where the server keeps its access token.
        .env("TILR_STATE_DIR", server.dir.join("state"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let listed = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 2, "{listed}");
    let fields: Vec<&str> = lines[1].split_whitespace().collect();
    assert_eq!(fields[0], pane_id.to_string());
    assert_eq!(fields[1], pid.to_string());
    assert_eq!(fields[2], "100x30");
    assert!(lines[1].ends_with("/bin/sh"), "{listed}");

    let version = Command::new(env!("CARGO_BIN_EXE_tilrd"))
        .args(["ps", "--url", &server.addr, "--server-version"])
        .env("TILR_STATE_DIR", server.dir.join("state"))
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        env!("TILR_VERSION")
    );
}

#[tokio::test]
async fn a_server_keeps_its_id_when_it_restarts() {
    let mut server = Server::start(&[]);
    let server_id = Client::connect_silent(&server)
        .await
        .hello()
        .await
        .server_id;
    assert!(server_id.parse::<uuid::Uuid>().is_ok(), "{server_id}");

    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let again = Server::start_in(
        server.dir.clone(),
        Path::new(env!("CARGO_BIN_EXE_tilrd")),
        &[],
    );
    let ack = Client::connect_silent(&again).await.hello().await;
    assert_eq!(ack.server_id, server_id);

    // Another machine, which here is another directory, is another server.
    let other = Server::start(&[]);
    let ack = Client::connect_silent(&other).await.hello().await;
    assert_ne!(ack.server_id, server_id);
}

#[tokio::test]
async fn two_clients_watch_and_type_into_the_same_pane() {
    let server = Server::start(&[]);
    let mut first = Client::connect(&server).await;
    let pane_id = first.open_pane().await;
    let mut second = Client::connect(&server).await;
    second.subscribe(pane_id).await;

    first
        .type_line(pane_id, "echo \"from the fi\"\"rst\"")
        .await;
    first.wait_row("from the first").await;
    second.wait_row("from the first").await;

    second
        .type_line(pane_id, "echo \"from the sec\"\"ond\"")
        .await;
    first.wait_row("from the second").await;
    second.wait_row("from the second").await;

    assert_eq!(first.list_panes().await[0].viewers, 2);
}

#[tokio::test]
async fn a_client_that_leaves_does_not_take_the_pane_with_it() {
    let server = Server::start(&[]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    let mut leaves = Client::connect(&server).await;
    leaves.subscribe(pane_id).await;
    leaves
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;
    assert_eq!(stays.list_panes().await[0].viewers, 2);

    drop(leaves);
    // The server tells the remaining client on its own.
    loop {
        let panes = stays.pushed_pane_list().await;
        if panes[0].viewers == 1 {
            break;
        }
    }
    assert_eq!(stays.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_client_that_stops_watching_gets_no_more_frames() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);

    client
        .send(Msg::Unsubscribe(Unsubscribe {
            pane_id: pane_id.to_string(),
        }))
        .await;
    assert_eq!(client.list_panes().await[0].viewers, 0);
    client.frame = None;
    client.type_line(pane_id, "echo more output").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.list_panes().await;
    assert!(client.frame.is_none());
}

#[tokio::test]
async fn resizing_a_pane_resizes_the_terminal_the_shell_sees() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .send(Msg::Resize(Resize {
            pane_id: pane_id.to_string(),
            cols: 50,
            rows: 10,
            cell_width_px: 0,
            cell_height_px: 0,
        }))
        .await;
    // A resize redraws the whole screen at the new size.
    let frame = client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.cols == 50 => Some(frame.clone()),
            _ => None,
        })
        .await;
    assert!(frame.full);
    assert_eq!(frame.rows, 10);

    let out = server.dir.join("size");
    assert_eq!(
        client
            .run(pane_id, &format!("stty size > '{}'", out.display()))
            .await,
        0
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "10 50\n");
    let panes = client.list_panes().await;
    assert_eq!((panes[0].cols, panes[0].rows), (50, 10));
}

#[tokio::test]
async fn a_client_built_for_another_protocol_version_is_turned_away() {
    let server = Server::start(&[]);
    assert_eq!(PROTOCOL_VERSION, 3);
    // Newer than the server, older than it, and one that sent no version.
    for version in [PROTOCOL_VERSION + 1, PROTOCOL_VERSION - 1, 0] {
        if version == PROTOCOL_VERSION {
            continue;
        }
        let mut client = Client::connect_silent(&server).await;
        client.hello_as(version).await;
        // The first thing it hears is why, with both versions; then nothing.
        let Some(server_message::Msg::Error(error)) = client.next().await else {
            panic!("version {version} was not refused");
        };
        assert_eq!(
            error.message,
            format!(
                "unsupported protocol version {version}: this server speaks {PROTOCOL_VERSION}"
            )
        );
        assert!(client.next().await.is_none());
    }
    // The server carries on for those who speak its version.
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn the_first_thing_a_client_hears_is_the_answer_to_its_hello() {
    let server = Server::start(&[]);
    let mut late = Client::connect_silent(&server).await;

    // Panes come and go while the client has not said hello yet.
    let mut other = Client::connect(&server).await;
    let pane_id = other.open_pane().await;
    other
        .type_line(pane_id, "printf '\\033]2;new title\\007'")
        .await;
    while other.pushed_pane_list().await[0].title != "new title" {}

    late.hello_as(PROTOCOL_VERSION).await;
    let first = late.next().await;
    assert!(
        matches!(first, Some(server_message::Msg::HelloAck(_))),
        "got {first:?} before the hello ack"
    );

    // From then on it is told about the panes like everyone else.
    other.type_line(pane_id, "exit").await;
    assert!(late.pushed_pane_list().await.is_empty());
}

#[tokio::test]
async fn a_client_that_skips_hello_is_turned_away() {
    let server = Server::start(&[]);
    let mut asks_first = Client::connect(&server).await;
    let pane_id = asks_first.open_pane().await;

    let mut client = Client::connect_silent(&server).await;
    client.type_line(pane_id, "echo \"got thr\"\"ough\"").await;
    let Some(server_message::Msg::Error(error)) = client.next().await else {
        panic!("a client that did not say hello was not refused");
    };
    assert_eq!(error.message, "expected Hello first");
    assert!(client.next().await.is_none());

    // What it sent did not reach the shell.
    assert_eq!(asks_first.run(pane_id, "true").await, 0);
    assert!(!asks_first.screen.has_row("got through"));
}

#[tokio::test]
async fn a_pane_does_not_get_the_service_managers_variables() {
    let dir = Server::scratch_dir();
    let (notify, env) = service_manager_env(&dir);
    let env: Vec<(&str, &str)> = env.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    let server = Server::start_with_env(dir, &[], &env);
    // The server itself still tells the service manager it is ready.
    notify
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut ready = [0; 64];
    let len = notify.recv(&mut ready).unwrap();
    assert_eq!(&ready[..len], b"READY=1");

    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "env > ~/pane-env").await, 0);
    let seen = std::fs::read_to_string(server.dir.join("pane-env")).unwrap();
    for (name, _) in &env {
        assert!(
            !seen
                .lines()
                .any(|line| line.starts_with(&format!("{name}="))),
            "{name} in the pane: {seen}"
        );
    }
    assert!(seen.lines().any(|line| line.starts_with("TILR_PANE=")));
}

/// Run, in the pane's shell, a program that ignores SIGHUP and writes its
/// pid to a file; in the background (`nohup … &`) if `background`. Its pid.
async fn start_ignoring_sighup(
    server: &Server,
    client: &mut Client,
    pane_id: PaneId,
    background: bool,
) -> u32 {
    let file = server.dir.join(format!("ignoring-{pane_id}"));
    let program = format!(
        "sh -c 'trap \"\" HUP; echo $$ > \"{}\"; exec sleep 1000'",
        file.display()
    );
    let line = if background {
        format!("nohup {program} >/dev/null 2>&1 &")
    } else {
        program
    };
    client.type_line(pane_id, &line).await;
    eventually("the program starts", || {
        std::fs::read_to_string(&file).is_ok_and(|pid| pid.ends_with('\n'))
    })
    .await;
    std::fs::read_to_string(&file)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// The pid of the process serving the pane.
async fn pane_process(client: &mut Client, pane_id: PaneId) -> u32 {
    let panes = client.list_panes().await;
    let pane = panes
        .iter()
        .find(|pane| pane.pane_id == pane_id.to_string())
        .unwrap();
    pane.pid
}

/// SIGTERM to the server; once it and the pane process are gone, whether
/// `pid` still runs.
async fn alive_after_the_server_stops(mut server: Server, pane_process: u32, pid: u32) -> bool {
    server.signal("-TERM");
    eventually("the server exits", || !server.is_running()).await;
    eventually("the pane process exits", || !process_alive(pane_process)).await;
    process_alive(pid)
}

#[tokio::test]
async fn closing_a_pane_ends_a_foreground_program_ignoring_sighup() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = start_ignoring_sighup(&server, &mut client, pane_id, false).await;
    let closed = std::time::Instant::now();
    client
        .send(Msg::ClosePane(ClosePane {
            pane_id: pane_id.to_string(),
        }))
        .await;
    client.pane_exit_code(pane_id).await;
    assert!(
        closed.elapsed() < Duration::from_secs(3),
        "{:?}",
        closed.elapsed()
    );
    assert!(!process_alive(pid), "the program is killed");
}

#[tokio::test]
async fn a_shell_ignoring_sighup_is_ended_when_the_server_stops() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // The pane's program itself ignores SIGHUP, and reads no input.
    client
        .send(Msg::CreatePane(CreatePane {
            cols: 80,
            rows: 24,
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "trap '' HUP; echo \"pid-$$\"; exec sleep 1000".into(),
            ],
            cwd: String::new(),
            request_id: 7,
        }))
        .await;
    let pane_id: PaneId = client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneCreated(created) if created.request_id == 7 => {
                Some(created.pane.as_ref().unwrap().pane_id.parse().unwrap())
            }
            _ => None,
        })
        .await;
    client.subscribe(pane_id).await;
    client.wait_screen("pid-").await;
    let text = client.screen.text();
    let pid: u32 = text
        .split("pid-")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .parse()
        .unwrap();
    let pane_process = pane_process(&mut client, pane_id).await;
    assert!(!alive_after_the_server_stops(server, pane_process, pid).await);
}

#[tokio::test]
async fn a_foreground_program_ignoring_sighup_is_ended_when_the_server_stops() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = start_ignoring_sighup(&server, &mut client, pane_id, false).await;
    let pane_process = pane_process(&mut client, pane_id).await;
    assert!(!alive_after_the_server_stops(server, pane_process, pid).await);
}

#[tokio::test]
async fn a_nohup_background_job_is_left_when_the_server_stops() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = start_ignoring_sighup(&server, &mut client, pane_id, true).await;
    let pane_process = pane_process(&mut client, pane_id).await;
    let left = alive_after_the_server_stops(server, pane_process, pid).await;
    // SAFETY: kill has no memory-safety preconditions; the pid is the job's.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    assert!(
        left,
        "a nohup job outlives the server, as a hangup leaves it"
    );
}
