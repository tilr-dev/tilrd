## 1. Build-script stdout path

- [x] 1.1 Read the stdout from `<OUT_DIR>/../run/stdout`, falling back to `<OUT_DIR>/../output`, in `crates/tilr/tests/build_script.rs` and `crates/tilrd/tests/build_script.rs`. Run them on Cargo 1.100 and on the MSRV: `cargo +1.100.0 nextest run -p tilr -p tilrd --test build_script --locked`, then the same with `+1.99`.
- [x] 1.2 The dt build exports whichever of the two paths exists (`itests/src/tilr_itests/main.py`), and itests/README.md documents both. Run `./itests/dt --only rust --filter build_script`.
- [x] 1.3 The `itests/tests/test_build.py` fixture reads either path. It plants a stale build-script result in both layouts (`build/stale-hash/output` and `build/stale/stale-hash/run/stdout`), and dt exports neither. Run `python3 -m unittest tests.test_build` in `itests/` with `RUSTUP_TOOLCHAIN=1.100.0` and with `RUSTUP_TOOLCHAIN=1.99`.

## 2. Vendored crates in the dt source snapshot

- [x] 2.1 Add `!vendor/` to `SOURCE_IGNORE` and document it in itests/README.md. The dt build stage (`cargo build --locked -p tilrd -p tilr`, run by `./itests/dt --only rust --filter build_script`) compiles the patched crates.

## 3. Verification and archive

- [x] 3.1 Run the AGENTS.md checks: `cargo fmt --all --check`; clippy with and without `--features tilrd/failpoints` on 1.100.0; `cargo +1.99 check --workspace --all-targets --features tilrd/failpoints --locked`; `cargo +1.100.0 nextest run --workspace --features tilrd/failpoints --locked`.
- [x] 3.2 Archive the change and list it under "Done" in openspec/ROADMAP.md; `openspec validate --all --strict`.
