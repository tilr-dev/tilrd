# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. SDKs

- [x] 1.1 [10.1] `scripts/install-tools.sh`: `deno` and `uv` pinned with SHA-256 for Linux and macOS, x86_64 and aarch64; `scripts/gen-bus-sdk.sh [--check]` (K4); verify `scripts/install-tools.sh /tmp/t deno uv && /tmp/t/deno --version && /tmp/t/uv --version` and the drift scenario of `bus-sdks` once by hand
- [x] 1.2 [10.2] `sdk/deno/` (Deno and browser; K1-K3), `deno.json`, `deno.lock`, test runtime and tests (K5); verify `deno test -A`, `deno fmt --check`, `deno lint` in `sdk/deno` with the tests named in the `bus-sdks` spec, except the two of 1.5
- [x] 1.3 [10.3] `sdk/python/` (`tilr_bus`, K1-K3), `pyproject.toml`, `uv.lock`, test runtime and tests including the cross-language ones; verify `uv run pytest` and `uv run ruff check` in `sdk/python`, except the two of 1.5
- [x] 1.4 [10.4] CI job `sdk` (two tiers), `scripts/sdk-check.sh`, AGENTS.md prerequisites and check list; verify `scripts/sdk-check.sh` passes locally and the job is in `.github/workflows/ci.yml` with the two tiers (`actionlint` if available, else review)
- [x] 1.5 Un-skip the scenarios that wait for the server side of other changes, in both SDKs: `instances_and_a_token_participant_connect_over_fd_socket_and_websocket` once `bus-instances` (`bus.toml`) and `bus-rights` (`bus.token`) land, `a_script_drives_a_terminal` once `bus-core-panes` and `pane-profiles` land; verify both pass in `scripts/sdk-check.sh` and drop "skipped" from their rows of USE_CASES.md and from the spec's "Verified by" lines. Done so: the instances are `autostart = false` with `rights.provide = ["test.*"]` and started with `bus.start`, which answers once each said `Hello` (no wait on the listing); the terminal script connects over WebSocket with a minted token holding only the pane calls, `pane.exited` and `pane = ["profile:demo", "access:own"]` (the scenario says so); the test harnesses keep the configuration directory under the Cargo target's `tmp/`, since the server refuses a profile below `/tmp`

## 2. Checks and archive

- [x] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`, `scripts/sdk-check.sh`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
