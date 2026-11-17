# Proposal: Sandboxed panes on Linux

Зависит от: pane-profiles, bus-core-services

Стартует после: bus-broker-core (the snapshot type of `pane-profiles` task 1.1 as designed); мержится после: pane-profiles, bus-core-services

Владеет: `crates/tilrd/src/sandbox/mod.rs` (the platform-neutral part: environment, refused
grants, fail closed, `TMPDIR` and cleanup, the `sandbox` feature check), `sandbox/linux.rs`,
the sandbox hook in `crates/tilrd/src/pty.rs` (`spawn_shell`), the Landlock and seccomp
dependencies, failpoints `sandbox-unavailable`, `sandbox-abi-4`, `sandbox-apply` (appended to
`fail.rs`), `tests/common/bus_sandbox.rs`, `crates/tilrd/tests/sandbox.rs`, itests UC-0S1,
README "Sandboxed panes" (created).

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` Part S, O3):
the sandboxed pane is done now; on Linux with Landlock (ABI ≥ 4) plus seccomp; `AF_UNIX`
always refused; grants over the state, runtime or configuration directory refused; fail closed.

## What Changes

- Start-up check (Landlock ABI ≥ 4, seccomp) and `sandbox` in `server.info.features`; a warning
  per pane below ABI 6 (S5, S9).
- Landlock ruleset and seccomp filter in the shell's `pre_exec`, base grants, `executables`
  resolution (S5); `PaneInfo.sandboxed`.
- Environment from scratch, no wrappers or shims, refused grants (S6), fail closed (S7), private
  `TMPDIR` and cleanup (S8). These parts are platform-neutral and used by `sandbox-macos`.
- A sandboxed pane is a normal terminal (S2).

## Capabilities

### New Capabilities
- `pane-sandbox`: sandboxed panes from sandboxed profiles, where the feature is offered, files,
  programs, programs made inside, network, a normal terminal, variables, no way to the server,
  fail closed, leftovers. ("The sandbox survives a reload" is `bus-reload`'s; the macOS
  requirements are `sandbox-macos`'s.)

## Non-goals

- macOS (`sandbox-macos`); sandboxing instances, `exec.run` or files; bubblewrap or user
  namespaces; network isolation beyond `none | client | server` (shared design).

## Impact

`sandbox/`, `pty.rs`, `fail.rs`, `Cargo.toml`/`Cargo.lock`/`deny.toml` (the Landlock and
seccomp crates), tests, itests, README. Feature flag `sandbox` (a server can run without it:
B14). Reload: the sandbox stays on the program's processes in the kernel; `PaneInfo.sandboxed`
crosses with `bus-reload` *(gap named by the split)*. Update from the previous release: nothing
released from `main` (O5).
