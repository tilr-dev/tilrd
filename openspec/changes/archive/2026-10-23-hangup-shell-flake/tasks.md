# Tasks

## 1. Flake

- [x] 1.1 `reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up`: failed once on macOS under load (a Linux container run and Dagger in parallel, 2026-10-18), "did not happen: the login shell exits with its terminal" at `tests/common/mod.rs:782` (`eventually`, 5 s). Find what holds the login shell past the hang-up under load (log when the terminal closes, the SIGHUP reaches the shell and the shell exits) and fix it, or widen the wait with the cause recorded. The historical implementation waits up to `TIMEOUT` for the shell's exit without an established cause (design.md).
- The widened wait does not establish a cause; a cause-based investigation and fix remain unresolved.

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
