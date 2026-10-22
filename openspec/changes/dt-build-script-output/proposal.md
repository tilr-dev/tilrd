# Preserve build-script output in dt Rust runtimes

## Why

The build-script regression tests read Cargo's saved stdout next to their
compile-time `OUT_DIR`. The dt build exports test executables, but its separate
runtime has no Cargo target cache, so both tests fail before checking the paths
watched by the build scripts.

## What Changes

Export the build-script stdout files reported by the test build's Cargo JSON
messages and restore them at their compiled paths in each Rust test runtime.
Keep the snapshot independent of the shared build cache and subsequent builds.

## Capabilities

- `itest-port-stand`: add a requirement for build-script output in Rust runtimes.

## Impact

Only dt build artifact packaging and Rust runtime setup change. The existing
`tilr` and `tilrd` build-script tests provide the regression: they must run in dt
and read the real test build's stdout without a mounted Cargo target cache.
