## MODIFIED Requirements

### Requirement: CI gates every change
Every pull request into `main` or `release/**`, every merge queue group, every push to `release/**`, a nightly run on `main` and a manual run SHALL run `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo nextest run --workspace --features tilrd/failpoints --locked` on Linux and macOS 15 (on a pull request, the quick tier); `cargo check` with failpoints at the MSRV 1.99; `cargo deny check`; `gitleaks git` over the whole history. A push to `main` SHALL NOT run it: every commit there passed its merge queue group.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

#### Scenario: A release branch fix is checked
- **WHEN** a fix pull request into `release/0.3` is opened or updated
- **THEN** the same checks run on it as on a pull request into `main`

#### Scenario: Build at the minimum supported Rust version
- **WHEN** CI runs the MSRV job
- **THEN** it installs Rust 1.99 and runs `cargo check --workspace --all-targets --features tilrd/failpoints --locked`

#### Scenario: A merged pull request
- **WHEN** the merge queue pushes a group's commit to `main`
- **THEN** no CI run starts for that push

### Requirement: CI scans secrets in the checked commit's history only
The CI gitleaks gate SHALL scan the whole history of the commit under check (the pushed release branch commit, the pull request's test merge, the merge queue group's commit or `main` in the nightly run) and no other branch fetched with it, so that a finding on another unmerged branch does not fail the check.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A leak on another pull request branch
- **WHEN** a commit with a gitleaks finding exists only on an unmerged pull request branch and the merge queue checks a group for `main`
- **THEN** the gitleaks job scans only the group commit's history and passes

#### Scenario: A leak in the checked history
- **WHEN** the pull request under check carries a commit with a gitleaks finding anywhere in its history
- **THEN** the gitleaks job fails

## ADDED Requirements

### Requirement: Pull requests run the quick test tier
On a pull request CI SHALL keep its job names, so the required checks of `main` report: `test (ubuntu-latest)` SHALL run the nextest profile `quick` of `.config/nextest.toml` (every test but the listed slow ones) and `test (macos-15)` SHALL skip its steps and succeed. Every other event SHALL run every test on Linux and macOS 15. Each test the `quick` profile leaves out SHALL exist.

**Verified by**: `release::ci_runs_every_gate_of_agents_md`, `release::the_quick_profile_skips_only_existing_tests` (static: `.github/workflows/ci.yml`, `.config/nextest.toml`).

#### Scenario: A pull request
- **WHEN** a pull request into `main` is opened or updated
- **THEN** `test (ubuntu-latest)` runs the tests with `NEXTEST_PROFILE=quick` and `test (macos-15)` reports success without building

#### Scenario: The merge queue
- **WHEN** the merge queue checks a group for `main`
- **THEN** both test jobs run every test with failpoints

#### Scenario: A renamed slow test
- **WHEN** a test the `quick` profile leaves out is renamed or removed
- **THEN** `release::the_quick_profile_skips_only_existing_tests` fails

### Requirement: CI restores build tools from a cache
CI jobs that need Zig, protoc or cargo-nextest SHALL cache the directory `scripts/install-tools.sh` installs into, keyed by the runner's OS and architecture and the script's hash, and SHALL run the script only when the cache misses; a change of a pinned version or checksum SHALL change the key.

**Verified by**: `release::build_tools_are_pinned_by_checksum_and_used_everywhere` (static: `.github/workflows/ci.yml`).

#### Scenario: Warm cache
- **WHEN** a run finds the cache for the current `scripts/install-tools.sh`
- **THEN** it restores the tools and skips the download

#### Scenario: A bumped tool
- **WHEN** a pinned version or checksum in `scripts/install-tools.sh` changes
- **THEN** the cache misses and the script downloads and checks the tools again

### Requirement: A nightly run checks main in full
CI SHALL run on `main` every night and on a manual run with the full tier and SHALL also run the black-box tests of `itests/` (`itests/dt` on a local Dagger engine, the Dagger CLI downloaded at the module's engine version and checked against a pinned SHA-256); the itests job SHALL NOT run on other events.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: The nightly run
- **WHEN** the schedule fires
- **THEN** CI runs on `main` with every test on Linux and macOS 15 and the itests job
