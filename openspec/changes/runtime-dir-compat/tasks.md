# Tasks

## 1. Old location on resume

- [ ] 1.1 Look up state in the old `$TMPDIR` location when the new one has none; verify with a failpoints/reload test that starts a server with `TMPDIR` set and a state file in the old place (`reload::a_reload_from_the_old_runtime_directory_keeps_the_panes`)
- [ ] 1.2 Verify with itests UC-060 from the previous release in a container with `TMPDIR` set and no `XDG_RUNTIME_DIR` (pid and panes kept)
- [ ] 1.3 README "Runtime directory" names both locations; USE_CASES.md entry; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
