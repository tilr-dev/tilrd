## 1. Build-script stdout path

- [x] 1.1 Read the stdout from `<OUT_DIR>/../run/stdout`, falling back to `<OUT_DIR>/../output`, in `crates/tilr/tests/build_script.rs` and `crates/tilrd/tests/build_script.rs`. Verified: `cargo +1.100.0 nextest run -p tilr -p tilrd --test build_script --locked` and the same with `+1.99`, 3/3 each.
- [x] 1.2 The dt build exports whichever of the two paths exists (`itests/src/tilr_itests/main.py`), and itests/README.md documents both. Verified: `./itests/dt --only rust --filter build_script`, 3/3.
- [x] 1.3 The `itests/tests/test_build.py` fixture reads either path. It plants a stale build-script result in both layouts (`build/stale-hash/output` and `build/stale/stale-hash/run/stdout`), and dt exports neither. Verified: `RUSTUP_TOOLCHAIN=1.100.0 python3 -m unittest tests.test_build` and `RUSTUP_TOOLCHAIN=1.99 python3 -m unittest tests.test_build` in `itests/` both pass.

## 2. Vendored crates in the dt source snapshot

- [x] 2.1 Add `!vendor/` to `SOURCE_IGNORE` and document it in itests/README.md. Verified: the dt build stage runs `cargo build --locked -p tilrd -p tilr` (`./itests/dt --only rust --filter build_script`).

## 3. Verification and archive

- [x] 3.1 AGENTS.md checks: `cargo fmt --all --check`; clippy with and without `--features tilrd/failpoints` on 1.100.0; `cargo +1.99 check --workspace --all-targets --features tilrd/failpoints --locked`; `cargo +1.100.0 nextest run --workspace --features tilrd/failpoints --locked` (553 passed, macOS).
- [ ] 3.2 CI on PR #134 is green: `test (ubuntu-latest)` on stable 1.100.
- [ ] 3.3 The merge queue is green: `test (ubuntu-latest)` and `test (macos-15)`. A `workflow_dispatch` or nightly run of `ci.yml` on `main` is green, including `itests` (`itests/dt --jobs 4`).
- [ ] 3.4 Archive the change and record its commit in openspec/ROADMAP.md. Verified: `openspec validate --all --strict`.
