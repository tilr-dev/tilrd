# Proposal: Sandboxed panes on macOS

Зависит от: sandbox-linux (merges after it; tasks 1.1 and 1.2 may be written at once)

Стартует после: bus-broker-core (tasks 1.1, 1.2 need nothing of the server); мержится после: sandbox-linux

Владеет: `crates/tilrd/src/sandbox/macos.rs`, the `tilrd sandbox-shell` subcommand (appended to
`crates/tilrd/src/main.rs`), the macOS branch of the sandbox hook in `pty.rs` (after
`sandbox-linux`), failpoint `sandbox-exec-missing`, the CI job `sandbox-macos` in
`.github/workflows/ci.yml`, the macOS part of README "Sandboxed panes".

## Why

Owner's decision of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` §S10, O3):
Seatbelt with a generated deny-default profile after the probe; the deprecated `sandbox-exec`
is an accepted risk, caught by CI on macOS 14, 15 and 26.

## What Changes

- SBPL profile generation with fixed rules and `(param …)` only (S10).
- `executables` resolution with Mach-O load commands and `#!` interpreters (S10).
- Spawn through `sandbox-exec` and the `tilrd sandbox-shell` trampoline with a ready pipe;
  start-up check; fail closed (S7, S10).
- CI job `sandbox-macos` (`macos-14`, `macos-15`, `macos-26`) in the merge queue, release
  branches and nightly, running every `sandbox::*` test and
  `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` (the latter once `bus-reload`
  added it).

## Capabilities

### New Capabilities
- `pane-sandbox`: the macOS profile carries paths as parameters only; the sandbox is tested on
  every supported macOS.

## Non-goals

- App Sandbox, a signed helper, a `DYLD_INSERT_LIBRARIES` interposer (S10, rejected).

## Impact

`sandbox/macos.rs`, `main.rs`, `pty.rs`, `fail.rs`, `ci.yml`, README. Feature `sandbox` on
macOS. Reload: the Seatbelt sandbox is attached to the shell's processes and inherited; the
pane process is not sandboxed; nothing new crosses (S10). Nothing released from `main` (O5).
