# Tasks

## 1. The drain's deadline

- [x] 1.1 A drain looks at the state without waking itself; verify unit tests `draining_a_connection_ends_at_its_deadline_without_spinning`, `the_broker_drain_ends_at_its_deadline_without_spinning` (crates/tilrd/src/bus/broker.rs); USE_CASES.md entries

## 2. Checks and archive

- [x] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`; verify all green
- [ ] 2.2 Archive the change; verify `openspec validate --all --strict`
