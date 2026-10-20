## 1. Contract

- [x] 1.1 Specify confirmed readiness, signal and timeout interruption, owned-PID checks, and cleanup in `specs/test-server-cleanup-stand/spec.md`; verify with `openspec validate test-server-cleanup-stand --strict`.

## 2. Stand

- [x] 2.1 Add ignored `cleanup_stand::server_ready_then_wait_for_interruption` fixture; verify it compiles with `cargo test -p tilrd --test cleanup_stand --no-run --locked`.
- [x] 2.2 Add `scripts/test_server_cleanup_stand.py` with exact-PID and macOS `pgrep` checks; verify real signal and timeout cases report surviving PIDs before TILR-153 and leave no owned server after exit.
- [x] 2.3 Add `scripts/test_server_cleanup_stand_selftest.py`; verify four integration cases pass, including surviving and clean children plus two failure paths.
- [x] 2.4 Document both commands in `README.md` and the ignored fixture in `USE_CASES.md`; verify the documented commands match the files and test names.

## 3. Checks

- [x] 3.1 Before marking the change done, run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`. Run `./itests/dt coverage` for the new requirement. Record SDK and sandbox workarounds in the task note.

## Review corrections
- [x] Rebase onto dev and refresh coverage from current specs.
- [x] Require an exit event, bound failure at 30 seconds, and fail on unavailable watchers.
- [x] Check the watcher contract deterministically and scope pgrep selftest to macOS.
- [ ] Verify macOS/Linux selftests and required checks.
