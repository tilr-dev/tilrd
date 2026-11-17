# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Test clients on the bus

- [ ] 1.1 [5.1, tests] `tilr-driver` and `crates/tilrd/tests/common/mod.rs` on the Rust client of `tilr-bus` (T10); every test in `crates/tilrd/tests/` moved to the bus with its name kept (the four protocol tests of the proposal stay on `tests/common/v1.rs`); the lanes' `tests/common/bus_*.rs` helpers kept or folded into `common` with their callers; verify `cargo nextest run --workspace --features tilrd/failpoints --locked` green with the same test names as before plus the new ones
- [ ] 1.2 [5.3] itests: `tilr-driver` and the scenarios on the bus, spec tags updated for renamed or moved requirements (`@case(spec=[...])`), `./itests/dt coverage` exits 0 (local, not committed); verify the itests CI job green

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
