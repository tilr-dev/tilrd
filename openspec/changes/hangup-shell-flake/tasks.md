# Tasks

## 1. Flake

- [x] 1.1 (TILR-117) `reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up`: failed once on macOS under load (a Linux container run and Dagger in parallel, 2026-10-18), "did not happen: the login shell exits with its terminal" at `tests/common/mod.rs:782` (`eventually`, 5 s); 3 of 3 green alone. Find what holds the login shell past the hang-up under load (log when the terminal closes, the SIGHUP reaches the shell and the shell exits) and fix it, or widen the wait with the cause recorded; verify 10 consecutive green full `cargo test --workspace --features tilrd/failpoints --locked` runs on macOS — Done: the wait for the shell's exit looks up to `TIMEOUT`, cause not found, reason recorded (design.md).
- Reopened by TILR-117 (2026-10-19): the widened wait is not a cause; the task rewrites 1.1 with the cause and the fix.

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
