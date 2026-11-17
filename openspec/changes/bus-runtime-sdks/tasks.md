# Tasks

## 1. Tooling

- [ ] 1.1 `scripts/install-tools.sh`: `deno` and `uv` at pinned versions with SHA-256 sums for Linux and macOS, x86_64 and aarch64; verify `scripts/install-tools.sh /tmp/t deno uv && /tmp/t/deno --version && /tmp/t/uv --version`
- [ ] 1.2 `scripts/gen-bus-sdk.sh [--check]` (D6); verify the scenario of "The generated schema matches bus.proto" by hand once and in CI

## 2. Deno

- [ ] 2.1 `sdk/deno/` moved from `runtime-tmp/runtimes/deno/bus.ts` and adapted (D2-D5), `deno.json`, `deno.lock`, test runtime and tests (D7); verify `deno test -A` in `sdk/deno` with the tests named in the `bus-sdks` spec; `deno fmt --check`, `deno lint`

## 3. Python

- [ ] 3.1 `sdk/python/` moved from `runtime-tmp/runtimes/python/bus.py` (without `AttachIdent`) and adapted (D2-D5), `pyproject.toml`, `uv.lock`, test runtime and tests including the cross-language one; verify `uv run pytest` and `uv run ruff check` in `sdk/python`

## 4. CI and docs

- [ ] 4.1 CI job `sdk` (D8), two tiers; `scripts/sdk-check.sh` for local runs; AGENTS.md prerequisites and check list; verify the job green on a PR and in the merge queue
- [ ] 4.2 README "Components": writing a component in Deno or Python (example `bus.toml` entry, handler, errors); USE_CASES.md lists the SDK tests; verify by review against the `bus-sdks` spec

## 5. Checks and archive

- [ ] 5.1 Run the checks from AGENTS.md (now including `scripts/sdk-check.sh`): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 5.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
