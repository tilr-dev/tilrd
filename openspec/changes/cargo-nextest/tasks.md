# Tasks

## 1. Install

- [x] 1.1 `scripts/install-tools.sh`: tool `nextest` (pinned version, a SHA-256 per platform, extracted into `DIR/cargo-nextest-<version>/`, linked as `DIR/cargo-nextest`), in the default set; verify `release::build_tools_are_pinned_by_checksum_and_used_everywhere` and a run on macOS
- [x] 1.2 `scripts/linux.Dockerfile` installs it and links `/usr/local/bin/cargo-nextest`; verify `cargo nextest --version` in the image

## 2. The command

- [x] 2.1 `scripts/linux.sh` default command, CI `test` job, AGENTS.md, README, USE_CASES.md and `openspec/config.yaml` name `cargo nextest run --workspace --features tilrd/failpoints --locked`; verify `release::ci_runs_every_gate_of_agents_md` and the new `release::the_test_command_is_the_same_everywhere`
- [ ] 2.2 The suite passes under nextest on macOS and in the Linux container

## 3. Checks and archive

- [ ] 3.1 The checks from AGENTS.md green; `openspec validate --all --strict`
- [ ] 3.2 Archive the change
