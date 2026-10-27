## MODIFIED Requirements

### Requirement: CI gates every change
Every pull request into `main` or `release/**`, every merge queue group and every push to `main` or `release/**` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo nextest run --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.99; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

#### Scenario: The release PR is checked
- **WHEN** the release PR into `release/0.3` is opened or updated, or a fix is cherry-picked into `release/0.3`
- **THEN** the same checks run on it as on a pull request into `main`

#### Scenario: Build at the minimum supported Rust version
- **WHEN** CI runs the MSRV job
- **THEN** it installs Rust 1.99 and runs `cargo check --workspace --all-targets --features tilrd/failpoints --locked`
