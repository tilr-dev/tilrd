# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. SDKs

- [ ] 1.1 [10.1] `scripts/install-tools.sh`: `deno` and `uv` pinned with SHA-256 for Linux and macOS, x86_64 and aarch64; `scripts/gen-bus-sdk.sh [--check]` (K4); verify `scripts/install-tools.sh /tmp/t deno uv && /tmp/t/deno --version && /tmp/t/uv --version` and the drift scenario of `bus-sdks` once by hand
- [ ] 1.2 [10.2] `sdk/deno/` (Deno and browser; K1-K3), `deno.json`, `deno.lock`, test runtime and tests (K5); verify `deno test -A`, `deno fmt --check`, `deno lint` in `sdk/deno` with the tests named in the `bus-sdks` spec
- [ ] 1.3 [10.3] `sdk/python/` (`tilr_bus`, K1-K3), `pyproject.toml`, `uv.lock`, test runtime and tests including the cross-language ones; verify `uv run pytest` and `uv run ruff check` in `sdk/python`
- [ ] 1.4 [10.4] CI job `sdk` (two tiers), `scripts/sdk-check.sh`, AGENTS.md prerequisites and check list; verify the job green on the PR and in the merge queue

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`, `scripts/sdk-check.sh`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
