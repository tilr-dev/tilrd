# Complete the Rust test runtime fixtures

## Why

Release regression tests read workflow and nextest configuration omitted from the
source snapshot. The release-key regression also compiles a build script, but
the separate runtime lacks a Rust compiler and linker.

## What Changes

Include the upstream-release and spec-change workflows and nextest configuration
in the filtered source snapshot. Provide the pinned build toolchain and a native
linker in Rust test runtimes, and document these dependencies.

## Capabilities

- `itest-port-stand`: complete release regression fixtures in Rust runtimes.

## Impact

Only the integration test stand and its documentation change. Scenario runtime
images remain separate from Rust compiler dependencies.
