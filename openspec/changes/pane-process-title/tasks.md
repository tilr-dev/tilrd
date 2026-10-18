# Tasks

## 1. Title

- [ ] 1.1 Set and update the title on Linux (`prctl`/argv rewrite) and macOS; verify with `panes::the_pane_process_title_follows_the_directory` (reads `/proc/<pid>/cmdline` or `ps -o command`)

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
