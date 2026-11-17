# Follow Cargo's build-dir layout and build the patched crates in dt

## Why

CI on `main` has been red since 2026-11-03 (TILR-911). There are two independent
causes:

- Stable Rust 1.100.0 (2026-11-09) changed Cargo's build-dir layout. CI installs
  `stable`; from run 43380943849 (2026-11-13) the log shows `rustc 1.100.0 (from rustc
  1.99.1)`. A build script's stdout now sits in `<OUT_DIR>/../run/stdout`, and
  `<OUT_DIR>/../output` no longer exists. Both `build_script::every_watched_file_exists_and_the_git_head_is_watched`
  tests read the old path and fail on Linux and macOS.
- TILR-839 (db57fb9f, #130) patches `libghostty-vt` and `libghostty-vt-sys` from
  `vendor/` through `[patch.crates-io]`. The dt source snapshot does not include
  `vendor/`, so the itests build fails with `patch location /src/tilrd/vendor/libghostty-vt-0.2.2
  does not contain packages matching libghostty-vt` (nightly run 41612845949, 2026-11-03).

## What Changes

- Both `build_script` tests read the build script's stdout from
  `<OUT_DIR>/../run/stdout` and fall back to `<OUT_DIR>/../output` on older Cargo
  (MSRV 1.99).
- The dt build exports the build-script stdout from whichever of the two paths
  exists. It restores the file at that same path in the Rust runtimes.
- The `itests/tests/test_build.py` fixture reads either path. Its stale-result decoy
  exists in both layouts.
- The dt source snapshot includes `vendor/`.
- itests/README.md documents both stdout paths and `vendor/` in the snapshot.

## Capabilities

- `itest-port-stand`: the build-script stdout path follows the Cargo version; the
  source snapshot includes the vendored patched crates.

## Non-goals

- Pinning the CI toolchain to a fixed Rust version.
- Bumping `RUST_IMAGE` (still `rust:1.99-bookworm`) or the MSRV.
- Other changes to which files the dt source snapshot includes.

## Impact

This changes only tests, the integration test stand and its documentation. The
protocol, reload state, update path and CDN layout stay the same. Nothing is
handed across a reload or an update. It is not a breaking change and adds no
feature flag.

## Implementation

Commit 9c52e70a on `task/TILR-911`, PR #134.
