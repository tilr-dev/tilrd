//! Failure diagnostics must not turn an ordinary test panic into an abort.

mod common;

use common::*;
use futures_util::FutureExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

#[tokio::test]
async fn a_failed_log_wait_unwinds_and_cleans_up_the_server() {
    const MODE: &str = "TILR_HARNESS_FAILURE_MODE";
    const DIR: &str = "TILR_HARNESS_FAILURE_DIR";
    const MISSING: &str = "deliberately absent log message";
    if let Ok(mode) = std::env::var(MODE) {
        let dir = PathBuf::from(std::env::var_os(DIR).unwrap());
        let server = Server::start_in(
            dir,
            Path::new(env!("CARGO_BIN_EXE_tilrd")),
            &["--no-auto-reload"],
        );
        let failure = match mode.as_str() {
            "async" => {
                AssertUnwindSafe(server.wait_logged_within(MISSING, Duration::ZERO))
                    .catch_unwind()
                    .await
            }
            "blocking" => catch_unwind(AssertUnwindSafe(|| {
                server.wait_logged_blocking_within(MISSING, Duration::ZERO);
            })),
            "poisoned" => {
                let log = Arc::clone(&server.log);
                assert!(
                    std::thread::spawn(move || {
                        let _guard = log.lock().unwrap();
                        panic!("deliberately poison the log");
                    })
                    .join()
                    .is_err()
                );
                panic!("deliberate failure with a poisoned log");
            }
            _ => panic!("unknown failure mode"),
        }
        .expect_err("missing-log wait unexpectedly succeeded");
        assert!(!server.log.is_poisoned(), "failed wait poisoned the log");
        eprintln!("log stayed usable");
        resume_unwind(failure);
    }

    let mut failures = Vec::new();
    for mode in ["async", "blocking", "poisoned"] {
        let dir = Server::scratch_dir();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a_failed_log_wait_unwinds_and_cleans_up_the_server",
                "--nocapture",
            ])
            .env(MODE, mode)
            .env(DIR, &dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = if mode == "poisoned" {
            "deliberate failure with a poisoned log"
        } else {
            MISSING
        };
        if output.status.code() != Some(101)
            || !stderr.contains("--- server log ---")
            || !stderr.contains(reason)
            || (mode != "poisoned" && !stderr.contains("log stayed usable"))
            || dir.exists()
        {
            failures.push(format!("{mode}: {}\n{stderr}", output.status));
        }
        let _ = std::fs::remove_dir_all(dir);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
