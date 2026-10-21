## MODIFIED Requirements

### Requirement: CI gates every change
Every pull request and push to `main` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo nextest run --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.90; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

### Requirement: Build tools are pinned and checked
`scripts/install-tools.sh DIR [zig|protoc|nextest]` SHALL install Zig 0.15.2, protoc 36.2 and cargo-nextest 0.9.149 into `DIR` for Linux or macOS on x86_64 or aarch64, each checked against a pinned SHA-256; CI, release jobs and the Linux build image SHALL use it.

**Verified by**: `release::build_tools_are_pinned_by_checksum_and_used_everywhere` (static: `scripts/install-tools.sh`, the workflows and `scripts/linux.Dockerfile`).

#### Scenario: Install both
- **WHEN** `scripts/install-tools.sh "$RUNNER_TEMP/tools"` runs on Linux x86_64
- **THEN** `DIR/zig`, `DIR/protoc` and `DIR/cargo-nextest` are in place after their checksums matched
