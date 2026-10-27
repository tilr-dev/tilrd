//! Deterministic regressions and process fixture for the external cleanup stand.

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[test]
fn owned_servers_exit_after_panic_signal_and_timeout() {
    for copied in [false, true] {
        let mut command = std::process::Command::new("python3");
        command
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/test_server_cleanup_stand.py"
            ))
            .arg("--executable")
            .arg(std::env::current_exe().unwrap())
            .args(["--cases", "panic", "panic-startup", "signal", "timeout"]);
        if copied {
            command.arg("--copied");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "copied={copied}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the external cleanup stand"]
async fn server_ready_then_wait_for_interruption() {
    use common::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixDatagram;

    let dir = PathBuf::from(
        std::env::var_os("TILR_STAND_DIR").expect("run through the external cleanup stand"),
    );
    let notify_path = dir.join("notify.sock");
    let notify = UnixDatagram::bind(&notify_path).unwrap();
    notify.set_read_timeout(Some(TIMEOUT)).unwrap();
    let path = notify_path.to_str().unwrap();
    let copied = std::env::var("TILR_STAND_COPY").unwrap_or_default() == "1";
    let exe = if copied {
        let exe = dir.join("tilrd");
        std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
        exe
    } else {
        PathBuf::from(env!("CARGO_BIN_EXE_tilrd"))
    };
    let startup = std::env::var("TILR_STAND_CASE").unwrap_or_default() == "panic-startup";
    let server = if startup {
        let mut command = Command::new(&exe);
        Server::environment(&mut command, &dir, "");
        command
            .args(["--listen", "127.0.0.1:0"])
            .env("NOTIFY_SOCKET", path)
            .stdin(Stdio::null())
            .stderr(Stdio::piped());
        Server::spawn_owned(command, dir)
    } else {
        Server::start_full(dir, &exe, &[], "", &[("NOTIFY_SOCKET", path)])
    };
    let mut message = [0; 64];
    let n = notify.recv(&mut message).expect("server READY=1 event");
    assert_eq!(&message[..n], b"READY=1");
    if !startup {
        let mut client = Client::connect(&server).await;
        let pane = client.open_pane().await;
        let shell = shell_pid(&server, &mut client, pane).await;
        println!("TILR_STAND_CHILD pid={}", client.list_panes().await[0].pid);
        println!("TILR_STAND_CHILD pid={shell}");
    }
    println!("TILR_STAND_READY pid={}", server.child.id());
    std::io::stdout().flush().unwrap();
    let mut input = [0];
    std::io::stdin().read_exact(&mut input).unwrap();
    assert_ne!(input, *b"p", "controlled fixture panic");
    drop(server);
}
