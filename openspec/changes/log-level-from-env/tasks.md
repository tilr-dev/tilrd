# Tasks

## 1. Filter

- [x] 1.1 Enable tracing-subscriber's `env-filter` feature; build the filter from `RUST_LOG` with the `info` default and the warning on an invalid value (`main.rs:257-261`); verify `cargo deny check` and `cargo build --locked` (Cargo.lock updated by cargo, not by hand)
- [x] 1.2 Test servers start with `RUST_LOG` removed (`tests/common/mod.rs`); verify `cargo test -p tilrd` output unchanged with `RUST_LOG=trace` set in the shell
- [x] 1.3 Test `local::rust_log_sets_the_log_level` (debug line with `RUST_LOG=debug`, none without, the warning for an invalid value); USE_CASES.md entry; verify it passes
- [x] 1.4 README "Running tilrd" documents `RUST_LOG` (syntax, default, that a reload keeps it and pane shells inherit it); verify by review against this proposal

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 2.2 `./itests/dt uc_016` green (no itests case added), then `./itests/dt coverage` exits 0 and local, git-ignored `openspec/coverage/` reports are regenerated without committing them
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
