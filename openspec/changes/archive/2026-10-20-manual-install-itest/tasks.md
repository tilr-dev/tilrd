# Tasks

## 1. Installation

- [x] 1.1 itests UC-007 (`uc_007.py::a_manual_install_from_the_cdn_layout_runs`, tag `installation/manual-install-from-the-cdn`; take the next free id if UC-007 is taken by then): read `latest`, fetch the archive, unpack to `~/.local/bin`, run `tilr`; itests/USE_CASES.md entry; verify `./itests/dt` UC-007 green

## 2. Checks

- [x] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 2.2 Refresh each MODIFIED requirement in `specs/` from the current `openspec/specs/` if a change archived since touched it; drop "(planned …)" from the **Verified by** lines; remove the tested items from the specs' Notes; verify `openspec validate manual-install-itest --strict`
