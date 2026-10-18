# Tasks

## 0. Decision

- [ ] 0.1 Owner defines the event kinds; write the `panes` / `protocol` deltas; drop `skip_specs`; verify with `openspec validate pane-activity-notifications --strict`

## 1. Implementation

- [ ] 1.1 Detect the chosen triggers in the pane process; verify with `panes::a_bell_in_an_unwatched_pane_is_reported` and one test per kind

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
