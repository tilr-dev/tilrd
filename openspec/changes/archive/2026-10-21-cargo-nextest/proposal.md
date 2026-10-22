# Proposal: run the test suite with cargo-nextest

## Why

`cargo test` runs the workspace's test binaries one after another; most of the suite is integration tests that start real servers, so one slow binary holds up the rest. cargo-nextest runs every test as its own process and the binaries in parallel instead of one after another, so the suite finishes in roughly half the time.

The test command is written down in several places (AGENTS.md, README, `scripts/linux.sh`, the Linux container, CI). The static check of CI (`release::ci_runs_every_gate_of_agents_md`) only stays honest if every place runs the same command, so all of them switch together, to exactly `cargo nextest run --workspace --features tilrd/failpoints --locked`.

No retries: a test that fails once and passes on a rerun is a flake, filed with its cause, not hidden by the runner.

## What Changes

- `scripts/install-tools.sh` installs a pinned cargo-nextest release (checked against pinned SHA-256 sums) next to Zig and protoc; `nextest` joins the default tool set.
- The Linux container (`scripts/linux.Dockerfile`) installs it through that script; `scripts/linux.sh` runs the nextest command when given no command.
- CI's `test` job (Linux and macOS 15) runs `cargo nextest run --workspace --features tilrd/failpoints --locked` instead of `cargo test`.
- AGENTS.md, README, USE_CASES.md and `openspec/config.yaml` name the nextest command.

## Capabilities

### Modified Capabilities
- `release-process`: see `specs/release-process/spec.md` (the CI gate runs the tests with cargo-nextest; the pinned build tools include cargo-nextest).

## Non-goals

- A nextest configuration file (`.config/nextest.toml`), retries, test groups or per-test timeouts.
- Changing any test.

## Impact

`scripts/install-tools.sh`, `scripts/linux.Dockerfile`, `scripts/linux.sh`, `.github/workflows/ci.yml`, `crates/tilrd/tests/release.rs` (static checks), AGENTS.md, README.md, USE_CASES.md, `openspec/config.yaml`. Developers need `cargo-nextest` on PATH (`scripts/install-tools.sh DIR nextest`). No change to the binaries, the protocol or a release.
