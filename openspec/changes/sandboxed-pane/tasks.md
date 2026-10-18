# Tasks

Deferred / optional: not scheduled now.

## 1. Experiment

- [ ] 1.1 Quarantine experiment on macOS (design D1); verify by a recorded result: a script and a `cargo`-built binary created inside the App Sandbox shell run (or the approach is reopened)

## 2. Mechanism (only after 1.1 passes)

- [ ] 2.1 Write the `panes` / `access-control` deltas and drop `skip_specs`; verify `openspec validate sandboxed-pane --strict`
- [ ] 2.2 Linux Landlock in `pre_exec`; verify with a test that a sandboxed pane cannot write outside its allowed directory and its child processes cannot either (kernel ≥ 5.13)
- [ ] 2.3 macOS sandboxed pane binary; verify with the same test on macOS and a reload keeping the restriction
- [ ] 2.4 README and USE_CASES.md entries; verify by review against this proposal

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
