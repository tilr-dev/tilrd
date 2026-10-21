//! Use cases: picking up a new executable without closing any shell.

mod common;

use common::*;

#[tokio::test]
async fn a_command_running_across_a_reload_is_reported_when_it_ends() {
    let server = Server::start(&[]);
    let (fifo, mut release) = activity_fifo(&server);
    let command = script(
        &server,
        "foreground.sh",
        &format!(
            "printf 'foreground-ready\\n'\nread -r _ < '{}'\n",
            fifo.display()
        ),
    );
    let mut client = Client::connect(&server).await;
    let pane = client.open_pane().await;
    client.type_line(pane, &command).await;
    client.wait_screen("foreground-ready").await;
    client = reload(&server, client).await;
    client.hello().await;
    client.subscribe(pane).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.pane_id == pane.to_string() => Some(()),
            _ => None,
        })
        .await;
    client.activities.clear();
    release_activity_fifo(&mut release);
    let activity = client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneActivity(a)
                if a.kind == tilr_proto::PaneActivityKind::CommandFinished as i32 =>
            {
                Some(a.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(activity.pane_id, pane.to_string());
    assert!(activity.at_ms > 0);
}

/// What must hold after any reload: the shell in `pane_id` is the same one,
/// with its state and what it printed, and the server works as before.
async fn assert_pane_survived(server: &mut Server, mut client: Client, pane_id: PaneId) {
    assert!(server.is_running());
    client.hello().await;
    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, pane_id.to_string());
    assert_eq!((panes[0].cols, panes[0].rows), (100, 30));

    client.subscribe(pane_id).await;
    // The screen from before the reload is still there.
    client.wait_screen("before the reload").await;
    // A variable set before the reload is still set: same shell process.
    client
        .type_line(pane_id, "echo \"kept=$KEPT pid=$$\"")
        .await;
    client.wait_screen("kept=yes pid=").await;
    let pid_line = |text: String| {
        text.lines()
            .rev()
            .find(|line| line.contains("pid="))
            .map(str::to_owned)
    };
    assert_eq!(
        pid_line(client.screen.text()),
        pid_line(std::fs::read_to_string(server.dir.join("pid")).unwrap())
    );

    // New panes open as before, and the clipboard still works.
    let new_pane = client.open_pane().await;
    assert_ne!(new_pane, pane_id);
    client.put_clipboard(1, "image/png", &image(500)).await;
    assert!(client.clipboard_ack().await.ok);
    let check = script(server, "check.sh", CLAUDE_CHECK_IMAGE);
    assert_eq!(client.run(new_pane, &check).await, 0);
    server
        .wait_logged("resumed after reload with 1 pane(s)")
        .await;
}

/// Open a pane and leave marks in its shell for [`assert_pane_survived`].
async fn pane_with_marks(server: &Server, client: &mut Client) -> PaneId {
    let pane_id = client.open_pane().await;
    let pid_file = server.dir.join("pid");
    client
        .type_line(
            pane_id,
            &format!(
                "KEPT=yes; echo 'before the reload'; echo \"kept=$KEPT pid=$$\" > '{}'",
                pid_file.display()
            ),
        )
        .await;
    client.wait_screen("before the reload").await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    pane_id
}

#[tokio::test]
async fn panes_survive_a_reload_asked_for_with_a_signal() {
    let mut server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    server.signal("-USR1");
    let client = reconnect(&server, client).await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn panes_survive_the_executable_being_replaced_on_disk() {
    // Run a copy of the server, then install a "new version" over it the
    // way an upgrade does: write next to it, rename into place.
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    let staged = dir.join("tilrd.new");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &staged).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    // Nobody tells the server: it notices on its own.
    let client = reconnect(&server, client).await;
    server.wait_logged("executable changed on disk").await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn a_replaced_executable_is_ignored_when_auto_reload_is_off() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let server = Server::start_in(dir.clone(), &exe, &["--no-auto-reload"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;

    let staged = dir.join("tilrd.new");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &staged).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    tokio::time::sleep(Duration::from_secs(4)).await;
    // Still the same connection, still the same server.
    assert_eq!(client.run(pane_id, "true").await, 0);
    assert!(!server.logged("reloading"));
}

/// How often the server looks at its executable on disk, and how long it
/// gives a new one to answer `state-formats` (`WATCH_INTERVAL` and
/// `CHECK_TIMEOUT` in src/reload.rs).
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// By when a server has logged what it makes of a new executable on disk:
/// it sees the file within two looks at it and gives it up to
/// `CHECK_TIMEOUT` to say what it reads, which under load the shell that
/// runs it can take most of. Waiting less gives up on a server still within
/// its bounds.
const JUDGED: Duration = WATCH_INTERVAL
    .saturating_mul(2)
    .saturating_add(CHECK_TIMEOUT)
    .saturating_add(Duration::from_secs(5));

#[tokio::test]
async fn a_broken_executable_is_not_reloaded_into() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;

    // An upgrade gone wrong: the new file does not run.
    let staged = dir.join("tilrd.new");
    std::fs::write(&staged, "#!/bin/sh\nexit 1\n").unwrap();
    let mut mode = std::fs::metadata(&staged).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&staged, mode).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    server.wait_logged_within("not reloading", JUDGED).await;
    // The server carries on with the code it has.
    assert!(server.is_running());
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn an_executable_that_reads_another_state_format_is_not_reloaded_into() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    // The formats the running server writes, as it reports them itself.
    let formats = Command::new(&exe)
        .env_remove("RUST_LOG")
        .arg("state-formats")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(formats.stdout).unwrap(),
        "supervisor-state 2\npane-state 3\nunix-socket 1\nreload-check 1\n"
    );

    // A version that runs fine but keeps the panes' state differently.
    // Taking over, it would misread what it is left and lose the shells.
    // Every other way a successor can disagree on formats is in the unit
    // test `reload::tests::check_formats_rejects_every_format_mismatch`.
    let staged = dir.join("tilrd.new");
    std::fs::write(
        &staged,
        "#!/bin/sh\necho 'supervisor-state 2\npane-state 2'\n",
    )
    .unwrap();
    let mut mode = std::fs::metadata(&staged).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&staged, mode).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    server
        .wait_logged_within("reads pane-state format 2, this server writes 3", JUDGED)
        .await;
    // Same server, same connection, same shell.
    assert!(server.is_running());
    assert_eq!(client.run(pane_id, "true").await, 0);

    // A version that reads this server's state is then reloaded into.
    let staged = dir.join("tilrd.new");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &staged).unwrap();
    std::fs::rename(&staged, &exe).unwrap();
    let client = reconnect(&server, client).await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn a_reload_shows_in_no_command_line_and_leaves_no_file_behind() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    let pane_pid = client.list_panes().await[0].pid;
    let before = (process_args(server.child.id()), process_args(pane_pid));
    assert!(before.1.contains(&format!("pane --id {pane_id} ")));

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    server
        .wait_logged("resumed after reload with 1 pane(s)")
        .await;

    // Same processes, looking the same.
    assert_eq!(client.list_panes().await[0].pid, pane_pid);
    let after = (process_args(server.child.id()), process_args(pane_pid));
    assert_eq!(after, before);
    // What they handed over was taken and removed.
    let runtime = server.dir.join("runtime");
    let left: Vec<_> = [runtime.clone(), runtime.join("panes")]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".state"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_reload_with_no_panes_open_works() {
    let mut server = Server::start(&[]);
    let client = Client::connect(&server).await;
    let mut client = reload(&server, client).await;
    assert!(server.is_running());
    assert!(client.list_panes().await.is_empty());
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn every_pane_keeps_its_own_shell_across_a_reload() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let mut panes = Vec::new();
    for name in ["one", "two", "three"] {
        let pane_id = client.open_pane().await;
        assert_eq!(client.run(pane_id, &format!("NAME={name}")).await, 0);
        panes.push((
            pane_id,
            name,
            shell_pid(&server, &mut client, pane_id).await,
        ));
    }

    let mut client = reload(&server, client).await;
    assert_eq!(client.list_panes().await.len(), 3);
    for (pane_id, name, pid) in panes {
        client.subscribe(pane_id).await;
        client.type_line(pane_id, "echo \"name=$NAME\"").await;
        client.wait_row(&format!("name={name}")).await;
        assert_eq!(shell_pid(&server, &mut client, pane_id).await, pid);
        client
            .send(Msg::Unsubscribe(Unsubscribe {
                pane_id: pane_id.to_string(),
            }))
            .await;
    }
}

#[tokio::test]
async fn output_printed_during_a_reload_is_not_lost() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Tall enough to show every line at once.
    let pane_id = client.open_pane_with(100, 60, "").await;
    client
        .type_line(
            pane_id,
            "i=1; while [ $i -le 40 ]; do echo \"tick-$i\"; i=$((i+1)); sleep 0.1; done",
        )
        .await;
    client.wait_row("tick-5").await;

    // The loop keeps printing while the supervisor and the pane process are
    // being replaced.
    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    client.wait_row("tick-40").await;
    for tick in 1..=40 {
        let row = format!("tick-{tick}");
        assert!(
            client.screen.has_row(&row),
            "missing {row}:\n{}",
            client.screen.text()
        );
    }
}

#[tokio::test]
async fn input_typed_just_before_a_reload_reaches_the_shell() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);

    let out = server.dir.join("typed");
    client
        .type_line(pane_id, &format!("echo once >> '{}'", out.display()))
        .await;
    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    // Delivered, and not twice.
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "once\n");
}

#[tokio::test]
async fn a_server_can_reload_again_and_again() {
    let mut server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "COUNT=0").await, 0);
    let pid = shell_pid(&server, &mut client, pane_id).await;

    for round in 1..=3 {
        client = reload(&server, client).await;
        client.subscribe(pane_id).await;
        assert_eq!(client.run(pane_id, "COUNT=$((COUNT+1))").await, 0);
        client.type_line(pane_id, "echo \"count=$COUNT\"").await;
        client.wait_row(&format!("count={round}")).await;
        assert_eq!(shell_pid(&server, &mut client, pane_id).await, pid);
    }
    assert!(server.is_running());
}

#[tokio::test]
async fn a_shell_that_exits_after_a_reload_still_reports_its_exit_code() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    client.type_line(pane_id, "exit 4").await;
    assert_eq!(client.pane_exit_code(pane_id).await, 4);
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn a_closing_pane_is_still_killed_after_a_reload() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // The pane's program ignores SIGHUP: only the kill at the close
    // deadline ends it.
    client
        .send(Msg::CreatePane(CreatePane {
            cols: 80,
            rows: 24,
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "trap '' HUP; echo \"pid-$$\"; sleep 1000".into(),
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
    let program: u32 = client
        .screen
        .text()
        .split("pid-")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .parse()
        .unwrap();

    client
        .send(Msg::ClosePane(ClosePane {
            pane_id: pane_id.to_string(),
        }))
        .await;
    // Once the server answers, the close is on its way to the pane ahead
    // of the reload, which comes well within the 2 s deadline.
    client.list_panes().await;
    let mut client = reload(&server, client).await;

    eventually("the program is killed", || !process_alive(program)).await;
    // The client sees the pane exit, unless that was before it connected.
    if client
        .list_panes()
        .await
        .iter()
        .any(|pane| pane.pane_id == pane_id.to_string())
    {
        client.pane_exit_code(pane_id).await;
    }
    assert!(client.list_panes().await.is_empty());
    server
        .wait_logged("resumed after reload with 1 pane(s)")
        .await;
}

#[tokio::test]
async fn the_cursor_under_origin_mode_is_where_it_was_after_a_reload() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // A scrolling region on rows 3 to 8 and origin mode: the cursor at 2;5
    // is on screen row 4. After a line (not echoed) the program prints the
    // rest of the word where the cursor is.
    client
        .send(Msg::CreatePane(CreatePane {
            cols: 40,
            rows: 12,
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "stty -echo; printf '\\033[3;8r\\033[?6h\\033[2;5Hmar'; read line; printf 'ker'; \
                 sleep 1000"
                    .into(),
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
    client.wait_row("    mar").await;
    assert_eq!(client.screen.rows[3].trim_end(), "    mar");

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    client.wait_row("    mar").await;
    client.type_line(pane_id, "go").await;
    client.wait_screen("ker").await;
    assert_eq!(
        client.screen.rows[3].trim_end(),
        "    marker",
        "screen:\n{}",
        client.screen.text()
    );
}

#[tokio::test]
async fn the_title_and_directory_of_a_pane_are_kept_across_a_reload() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let cwd = server.dir.display().to_string();
    let pane_id = client.open_pane_with(100, 30, &cwd).await;
    client
        .type_line(pane_id, "printf '\\033]2;my title\\007'")
        .await;
    let before = loop {
        let panes = client.pushed_pane_list().await;
        if panes[0].title == "my title" {
            break panes[0].clone();
        }
    };
    assert_eq!(before.cwd, cwd);

    let mut client = reload(&server, client).await;
    let after = client.list_panes().await.remove(0);
    assert_eq!(after.title, "my title");
    assert_eq!(after.cwd, cwd);
    assert_eq!(after.created_at_ms, before.created_at_ms);
    assert_eq!(after.command, before.command);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn the_pane_process_title_is_set_again_after_a_reload() {
    let TitleTest {
        server,
        controller,
        mut client,
        pane,
        shell,
        pid,
        hook,
        points,
    } = TitleTest::start().await;
    client
        .type_line(pane, "cd /tmp; printf '\\033]7;file://localhost/tmp\\007'")
        .await;
    let suffix = format!("[shell {shell} in /tmp]");
    let point = wait_process_title(&controller, &hook, pid, &suffix).await;
    let before = process_args(pid);
    assert!(before.contains("--cols 100 --rows 30"));
    let resumed = format!("pane-resume-live:{pane}");
    std::fs::write(&points, format!("{resumed}=pause")).unwrap();
    point.release().await;

    let mut client = reload(&server, client).await;
    let live = controller.wait(&resumed).await;
    let after_pid = client.list_panes().await[0].pid;
    std::fs::write(&points, format!("{hook}=pause")).unwrap();
    live.release().await;
    let point = wait_process_title(&controller, &hook, after_pid, &suffix).await;
    assert_eq!(process_args(after_pid), before);
    std::fs::remove_file(points).unwrap();
    point.release().await;
    client.subscribe(pane).await;
    assert_eq!(client.run(pane, "test \"$PWD\" = /tmp").await, 0);
}

#[tokio::test]
async fn stopping_the_server_ends_the_shells() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;
    assert!(process_alive(pid));

    server.signal("-TERM");
    eventually("the shell exits", || !process_alive(pid)).await;
}

#[tokio::test]
async fn stopping_the_server_after_a_reload_ends_the_shells() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    assert!(process_alive(pid));

    server.signal("-TERM");
    eventually("the shell exits", || !process_alive(pid)).await;
}

// What follows makes a reload go wrong at a chosen place, see `src/fail.rs`.

/// A copy of the server in a directory of its own, where things go wrong as
/// `failpoints` says.
#[cfg(feature = "failpoints")]
fn failing_server(failpoints: &str) -> (Server, PathBuf) {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    (Server::start_failing(dir, &exe, &[], failpoints), exe)
}

/// Install a "new version" over the executable: another file that runs the
/// same.
#[cfg(feature = "failpoints")]
fn replace(exe: &Path) {
    let staged = exe.with_extension("new");
    let mut bytes = std::fs::read(exe).unwrap();
    bytes.push(0);
    std::fs::write(&staged, bytes).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(&staged, mode).unwrap();
    std::fs::rename(&staged, exe).unwrap();
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_reload_that_fails_before_the_handover_leaves_everything_as_it_was() {
    let (mut server, _) = failing_server("reload-before-exec=error");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    server.wait_logged("reload failed").await;

    // Not even the connection was dropped: the same client carries on.
    assert!(server.is_running());
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
    assert_eq!(shell_pid(&server, &mut client, pane_id).await, pid);
    // And a new pane opens as before.
    let other = client.open_pane().await;
    assert_eq!(client.run(other, "true").await, 0);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has() {
    let (mut server, _) = failing_server("pane-reload-before-exec=error");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    // The supervisor reloads; the pane process stays the one it was.
    let client = reload(&server, client).await;
    server.wait_logged("reload failed").await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on() {
    let (mut server, _) = failing_server("");
    let mut client = Client::connect(&server).await;
    let lost = client.open_pane().await;
    std::fs::write(
        server.dir.join("failpoints"),
        format!("pane-resume-live:{lost}=panic"),
    )
    .unwrap();
    let lost_shell = shell_pid(&server, &mut client, lost).await;
    let kept = client.open_pane().await;
    assert_eq!(client.run(kept, "KEPT=yes").await, 0);
    let kept_shell = shell_pid(&server, &mut client, kept).await;

    let mut client = reload(&server, client).await;
    server.wait_logged("pane process died").await;
    assert!(server.is_running());
    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, kept.to_string());
    client.subscribe(kept).await;
    assert_eq!(client.run(kept, "test \"$KEPT\" = yes").await, 0);
    assert_eq!(shell_pid(&server, &mut client, kept).await, kept_shell);
    // The shell of the lost pane does not linger without a pane.
    eventually("the shell of the lost pane exits", || {
        !process_alive(lost_shell)
    })
    .await;
}

/// The limit of reloading in place: once the new version has taken over
/// there is no old one to go back to. What the server checks beforehand
/// (the formats it reads, then a check run of its resume: the tests below)
/// is all that stands between a version that crashes on taking over and the
/// shells; this one passes the check and crashes after it.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_version_that_crashes_taking_over_ends_the_server_and_its_shells() {
    for failure in ["panic", "error"] {
        let (mut server, _) = failing_server(&format!("supervisor-resume-live={failure}"));
        let mut client = Client::connect(&server).await;
        let pane_id = client.open_pane().await;
        let pid = shell_pid(&server, &mut client, pane_id).await;

        server.signal("-USR1");
        eventually("the server ends", || !server.is_running()).await;
        // No shell is left running where nobody can reach it.
        eventually("the shell exits", || !process_alive(pid)).await;
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_server_killed_in_the_middle_of_a_reload_leaves_no_shell_behind() {
    let (mut server, _) = failing_server("reload-before-exec=kill");
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    eventually("the server ends", || !server.is_running()).await;
    eventually("the shell exits", || !process_alive(pid)).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_executable_replaced_again_during_a_reload_costs_no_shell() {
    // The reload takes a second and a half, so the second replacement lands
    // in the middle of it.
    let (mut server, exe) = failing_server("reload-before-exec=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    replace(&exe);
    // The server first runs the new file to ask what it reads, which takes
    // macOS seconds for a file it has not seen before.
    let started = tokio::time::Instant::now();
    while !server.logged("reloading") {
        assert!(started.elapsed() < TIMEOUT, "the reload did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    replace(&exe);

    let client = reconnect(&server, client).await;
    assert_pane_survived(&mut server, client, pane_id).await;
    // Whatever the server makes of the second file, it stays up with its
    // panes.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let mut client = Client::connect(&server).await;
    assert_eq!(client.list_panes().await.len(), 2);
    assert!(!server.logged("pane process died"));
}

// The successor shows it can resume before the exec: a check run of the
// executable on disk with the server's command line, configuration and a
// copy of its state.

/// What must hold after a refused reload: the same server process, the same
/// connection, the same shell with its variables, no state or check file
/// left.
async fn assert_nothing_changed(
    server: &mut Server,
    client: &mut Client,
    pane_id: PaneId,
    pid: u32,
) {
    assert!(server.is_running());
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
    assert_eq!(shell_pid(server, client, pane_id).await, pid);
    assert!(!server.logged("resumed after reload"));
    assert!(
        !server.logged("reloading exe="),
        "nothing was stopped for it"
    );
    let runtime = server.dir.join("runtime");
    let left: Vec<String> = [runtime.clone(), runtime.join("panes")]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".state"))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

const REFUSED: &str = "not reloading: the new executable could not resume";

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_successor_that_panics_resuming_is_not_reloaded_into() {
    let (mut server, _) = failing_server("supervisor-resume=panic");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let pid = shell_pid(&server, &mut client, pane_id).await;
    let server_pid = server.child.id();

    server.signal("-USR1");
    server.wait_logged(REFUSED).await;
    // The reason is logged.
    server.wait_logged("failpoint supervisor-resume").await;
    assert_eq!(server.child.id(), server_pid);
    assert_nothing_changed(&mut server, &mut client, pane_id, pid).await;
}

/// The running server's command line has what an older or broken release
/// does not accept (`tilr` starts its server with `--socket`): that release
/// would exit on it after the exec, with every shell.
#[tokio::test]
async fn a_successor_that_rejects_the_command_line_is_not_reloaded_into() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    let real = dir.join("tilrd-real");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &real).unwrap();
    // A short path: a socket's is limited to about a hundred bytes.
    let socket = PathBuf::from(format!(
        "/tmp/tilr-rc-{}-{}.sock",
        std::process::id(),
        dir.file_name()
            .unwrap()
            .to_string_lossy()
            .rsplit('-')
            .next()
            .unwrap()
    ));
    let socket_arg = socket.display().to_string();
    let mut server = Server::start_in(dir.clone(), &exe, &["--socket", &socket_arg]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    // Reads the same state and says it can be checked, but knows no
    // --socket, as clap says it.
    let staged = dir.join("tilrd.new");
    std::fs::write(
        &staged,
        format!(
            "#!/bin/sh\n\
             for arg; do\n\
             [ \"$arg\" = --socket ] && {{ echo \"error: unexpected argument '--socket' found\" >&2; exit 2; }}\n\
             done\n\
             exec '{}' \"$@\"\n",
            real.display()
        ),
    )
    .unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(&staged, mode).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    server.signal("-USR1");
    server.wait_logged(REFUSED).await;
    // The log names the argument.
    server.wait_logged("unexpected argument '--socket'").await;
    assert_nothing_changed(&mut server, &mut client, pane_id, pid).await;
    assert!(socket.exists(), "the socket stays");
    drop(server);
    let _ = std::fs::remove_file(&socket);
}

/// A configuration file the running server read but the executable on disk
/// refuses (a key it does not know: an older release, or an edit made
/// since the start): the successor would stop on it after the exec.
#[tokio::test]
async fn a_successor_that_refuses_the_configuration_is_not_reloaded_into() {
    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/server.toml"), "socks = false\n").unwrap();
    let mut server = Server::start_in(dir.clone(), Path::new(env!("CARGO_BIN_EXE_tilrd")), &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    std::fs::write(
        dir.join("config/server.toml"),
        "socks = false\nsocsk = true\n",
    )
    .unwrap();
    server.signal("-USR1");
    server.wait_logged(REFUSED).await;
    // The log says why.
    server.wait_logged("unknown field `socsk`").await;
    assert_nothing_changed(&mut server, &mut client, pane_id, pid).await;

    // Fixed, the next reload goes through.
    std::fs::write(dir.join("config/server.toml"), "socks = false\n").unwrap();
    let client = reload(&server, client).await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_successor_that_never_answers_the_check_is_not_reloaded_into() {
    let (mut server, _) = failing_server("supervisor-resume=sleep:20000");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    // The server serves while it waits for the answer.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
    assert!(!server.logged(REFUSED));
    server
        .wait_logged_within("did not answer within 10 s", Duration::from_secs(30))
        .await;
    server.wait_logged(REFUSED).await;
    assert_nothing_changed(&mut server, &mut client, pane_id, pid).await;
}

/// A check that takes its time and passes: the server serves meanwhile,
/// then reloads in place, keeping its pid.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_successor_is_checked_while_the_server_serves_and_then_reloaded_into() {
    let (mut server, _) = failing_server("supervisor-resume=pause");
    let pauses = PauseController::bind(&server.dir);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let server_pid = server.child.id();

    server.signal("-USR1");
    let paused = pauses.wait("supervisor-resume").await;
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
    assert!(!server.logged("resumed after reload"));
    paused.release().await;
    let client = reconnect(&server, client).await;
    assert_eq!(server.child.id(), server_pid);
    assert_pane_survived(&mut server, client, pane_id).await;
    assert!(!server.logged(REFUSED));
}

/// The pane id is only known after startup; each hit needs a fresh release.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_runtime_named_pause_holds_a_pane_check_until_released() {
    let (mut server, _) = failing_server("");
    let pauses = PauseController::bind(&server.dir);
    let mut client = Client::connect(&server).await;
    let pane = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane).await;
    let name = format!("pane-resume:{pane}");
    let resumed = format!("resumed after reload pane=\"{pane}\"");
    std::fs::write(server.dir.join("failpoints"), format!("{name}=pause")).unwrap();

    for hit in 1..=2 {
        server.signal("-USR1");
        client = reconnect(&server, client).await;
        let paused = pauses.wait(&name).await;
        // The supervisor has resumed, but the pane's check is held. Its
        // shell stays alive and the supervisor still answers requests.
        assert!(server.is_running());
        assert!(process_alive(pid));
        assert_eq!(client.list_panes().await[0].pane_id, pane.to_string());
        paused.release().await;
        tokio::time::timeout(TIMEOUT, async {
            loop {
                let next = server.log_changed.notified();
                tokio::pin!(next);
                next.as_mut().enable();
                if count_logged(&server, &resumed) >= hit {
                    break;
                }
                next.await;
            }
        })
        .await
        .expect("pane did not resume after release");
        client.subscribe(pane).await;
        assert_eq!(shell_pid(&server, &mut client, pane).await, pid);
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_disconnected_pause_controller_refuses_the_reload() {
    let (mut server, _) = failing_server("supervisor-resume=pause");
    let pauses = PauseController::bind(&server.dir);
    let mut client = Client::connect(&server).await;
    let pane = client.open_pane().await;
    server.signal("-USR1");
    drop(pauses.wait("supervisor-resume").await);
    server.wait_logged(REFUSED).await;
    server.wait_logged("wait for release").await;
    assert!(server.is_running());
    assert_eq!(client.run(pane, "true").await, 0);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_pane_whose_successor_fails_the_check_carries_on() {
    let (mut server, _) = failing_server("pane-resume=panic");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    // The supervisor's check passes and it reloads; the pane process's
    // fails, and it stays the one it was.
    let client = reload(&server, client).await;
    server
        .wait_logged("the new executable could not resume")
        .await;
    server.wait_logged("reload failed").await;
    server.wait_logged("failpoint pane-resume").await;
    // The same shell: assert_pane_survived compares its pid too.
    assert_pane_survived(&mut server, client, pane_id).await;
    assert!(!server.logged("pane process died"));
}

// Signals that arrive while the successor starts, before it has its
// handlers, are held across the exec for it.

#[cfg(feature = "failpoints")]
fn count_logged(server: &Server, needle: &str) -> usize {
    server
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains(needle))
        .count()
}

/// Start a reload and come back once the successor is starting, sleeping
/// before its handlers are in place (`supervisor-resume-live=sleep:...`).
#[cfg(feature = "failpoints")]
async fn in_the_successor_starting(server: &Server) {
    server.signal("-USR1");
    server.wait_logged("reloading exe=").await;
    // The exec follows within milliseconds.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn signals_arriving_while_the_successor_starts_are_held_for_it() {
    let (mut server, _) = failing_server("supervisor-resume-live=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let server_pid = server.child.id();

    in_the_successor_starting(&server).await;
    // Each of them, unheld, would end the server here.
    server.signal("-HUP");
    server.signal("-USR1");
    let client = reconnect(&server, client).await;
    drop(client);
    let started = tokio::time::Instant::now();
    while count_logged(&server, "resumed after reload with") < 2 {
        assert!(started.elapsed() < TIMEOUT, "no second reload");
        assert!(server.is_running(), "the server ended");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    server.wait_logged("SIGHUP ignored").await;
    server.wait_logged("SIGUSR1 came during the reload").await;
    assert_eq!(server.child.id(), server_pid);

    let survivor = Client::connect(&server).await;
    assert_pane_survived(&mut server, survivor, pane_id).await;
    let mut client = Client::connect(&server).await;
    let new_pane = client.open_pane().await;
    // A shell started after the held signals has none of them blocked: a
    // hangup ends it (128 + SIGHUP).
    assert_eq!(
        client
            .run(new_pane, "sh -c 'kill -HUP $$; echo survived'")
            .await,
        129
    );
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sigterm_while_the_successor_starts_stops_the_server_cleanly() {
    let (mut server, _) = failing_server("supervisor-resume-live=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    in_the_successor_starting(&server).await;
    server.signal("-TERM");
    let started = tokio::time::Instant::now();
    let status = loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            break status;
        }
        assert!(started.elapsed() < TIMEOUT, "the server keeps running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        status.success(),
        "a clean shutdown, not the signal: {status}"
    );
    server.wait_logged("shutting down").await;
    eventually("the shell exits", || !process_alive(pid)).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sigint_while_the_successor_starts_stops_the_server_cleanly() {
    let (mut server, _) = failing_server("supervisor-resume-live=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;
    let socket = server.dir.join("runtime/tilrd.sock");
    assert!(socket.exists(), "the server has its Unix socket");

    in_the_successor_starting(&server).await;
    server.signal("-INT");
    let started = tokio::time::Instant::now();
    let status = loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            break status;
        }
        assert!(started.elapsed() < TIMEOUT, "the server keeps running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        status.success(),
        "a clean shutdown, not the signal: {status}"
    );
    server.wait_logged("SIGINT came during the reload").await;
    server.wait_logged("shutting down").await;
    eventually("the shell exits", || !process_alive(pid)).await;
    assert!(!socket.exists(), "the socket is left");
    assert!(
        !tilr_proto::local::lock_path(&socket).exists(),
        "the lock file is left"
    );
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sigterm_while_the_supervisor_execs_waits_for_the_successor() {
    // The supervisor stops right before its exec; any of its threads could
    // take a signal there, and the one that did would hand it to an image
    // about to be replaced.
    let (mut server, _) = failing_server("reload-exec=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    // Logged at the failpoint, after the signals are held.
    server.wait_logged("failpoint reload-exec: sleeping").await;
    server.signal("-TERM");
    let started = tokio::time::Instant::now();
    let status = loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            break status;
        }
        assert!(started.elapsed() < TIMEOUT, "the server keeps running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        status.success(),
        "a clean shutdown, not the signal: {status}"
    );
    server.wait_logged("SIGTERM came during the reload").await;
    server.wait_logged("shutting down").await;
    eventually("the shell exits", || !process_alive(pid)).await;
}

/// Wait for the server to end on its own.
#[cfg(feature = "failpoints")]
async fn exit_status(server: &mut Server) -> std::process::ExitStatus {
    let started = tokio::time::Instant::now();
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            return status;
        }
        assert!(started.elapsed() < TIMEOUT, "the server keeps running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sigterm_while_the_reload_prepares_its_exec_stops_the_server() {
    // The supervisor stops after it read its events a last time, before
    // the panes' writers detach and the exec: the event a SIGTERM turns
    // into is not read there.
    let (mut server, _) = failing_server("reload-after-drain=sleep:1500");
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    server
        .wait_logged("failpoint reload-after-drain: sleeping")
        .await;
    server.signal("-TERM");
    let status = exit_status(&mut server).await;
    assert!(
        status.success(),
        "a clean shutdown, not the signal: {status}"
    );
    // The exec went ahead.
    server.wait_logged("resumed after reload").await;
    server.wait_logged("SIGTERM came during the reload").await;
    server.wait_logged("shutting down").await;
    eventually("the shell exits", || !process_alive(pid)).await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_broken_pipe_after_a_failed_exec_does_not_kill_the_server() {
    // The exec fails in execvp, after the standard library set SIGPIPE to
    // its default for the new image. The test reads the server's stderr
    // itself, and stops reading it once the reload failed.
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut command = Command::new(&exe);
    command.args(["--listen", "127.0.0.1:0"]);
    Server::environment(&mut command, &dir, "reload-exec-missing=error");
    command.stdin(Stdio::null()).stderr(Stdio::piped());
    let mut server = Server::spawn_owned(command, dir);
    let mut lines = BufReader::new(server.child.stderr.take().unwrap()).lines();
    let log = Arc::new(Mutex::new(Vec::new()));
    let addr = loop {
        let line = lines.next().expect("the server did not start").unwrap();
        let addr = line
            .split("listening on ")
            .nth(1)
            .map(|a| a.trim().to_owned());
        log.lock().unwrap().push(line);
        if let Some(addr) = addr {
            break addr;
        }
    };
    let sink = Arc::clone(&log);
    let reader = std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            let failed = line.contains("reload failed");
            sink.lock().unwrap().push(line);
            if failed {
                // Dropping the lines closes the pipe's read end.
                break;
            }
        }
    });
    server.addr = addr;
    server.log = log;
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pid = shell_pid(&server, &mut client, pane_id).await;

    server.signal("-USR1");
    eventually("the reload fails", || reader.is_finished()).await;
    reader.join().unwrap();
    assert!(server.logged("exec "), "{:?}", server.log.lock().unwrap());

    // A new connection is logged, to a pipe nobody reads any more.
    let mut other = Client::connect(&server).await;
    other.hello().await;
    assert!(server.is_running(), "ended by the broken pipe");
    assert_eq!(client.run(pane_id, "true").await, 0);
    assert_eq!(shell_pid(&server, &mut client, pane_id).await, pid);
    assert!(server.is_running());
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_storm_of_reload_signals_costs_no_shell() {
    // A short pause before the handlers: some of the signals land there.
    let (mut server, _) = failing_server("supervisor-resume-live=sleep:100");
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;
    let server_pid = server.child.id();
    drop(client);

    for _ in 0..150 {
        // SAFETY: kill has no memory-safety preconditions.
        unsafe { libc::kill(server_pid as libc::pid_t, libc::SIGUSR1) };
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Let the last reloads finish.
    let mut seen = count_logged(&server, "resumed after reload with");
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let now = count_logged(&server, "resumed after reload with");
        if now == seen {
            break;
        }
        seen = now;
    }
    assert!(server.is_running());
    assert_eq!(server.child.id(), server_pid);
    assert!(seen >= 2, "reloads: {seen}");
    let client = Client::connect(&server).await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn a_second_server_keeps_its_host_socket_across_a_reload() {
    let first = Server::start(&[]);
    let second = start_second(&first);
    let mut client = Client::connect(&second).await;
    let pane_id = client.open_pane().await;
    let sock = pane_sock(&second, &mut client, pane_id).await;
    assert!(
        sock.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(&format!("-{}.sock", second.child.id())),
        "{}",
        sock.display()
    );

    let mut client = reload(&second, client).await;
    client.subscribe(pane_id).await;
    client
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;
    // The pane from before the reload still reaches this server's clients.
    assert_eq!(client.run(pane_id, "tilr edit kept.md").await, 0);
    let event = loop {
        if let Some(event) = client.host_events.pop_front() {
            break event;
        }
        client.next().await.expect("connection closed");
    };
    assert_eq!(event.pane_id, pane_id.to_string());
    assert!(sock.exists());
    second.wait_logged("reload").await;
}
