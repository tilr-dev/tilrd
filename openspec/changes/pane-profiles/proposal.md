# Proposal: Launch profiles for panes

Зависит от: bus-core-panes (task 1.1 needs nothing and may start at once)

Стартует после: now (task 1.1); task 1.2 after bus-broker-core; мержится после: bus-core-panes

Владеет: `crates/tilrd/src/profile.rs`, `crates/tilrd/src/bus/core/profiles.rs`, the
`pane.create {profile}` path in `bus/core/panes.rs` (after `bus-core-panes`), the
`profiles_dir` key in `crates/tilrd/src/config.rs`, `tests/common/bus_profiles.rs`,
`crates/tilrd/tests/profiles.rs`, README "Launch profiles".

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` Part P,
O3): panes get named launch profiles in `profiles_dir`; the request's command overrides the
profile's and the profile's sandbox stays; files checked like ssh's `StrictModes`; no feature
flag.

## What Changes

- `server.toml` key `profiles_dir` (default `profiles/` next to the `server.toml` in use; P1).
- Profile files `<name>.toml`: names never paths, StrictModes check, strict TOML, path and
  `[env]` rules, `[sandbox]` parsed (P2, P3).
- `pane.create {profile}` reads the profile on every create, the request overrides command and
  cwd, the resolved snapshot is the pane's (P4, P5, P7); `PaneInfo.profile`.
- `pane.profiles` lists the profiles with their errors (P6).
- Sandboxed profiles are refused until `sandbox-linux` / `sandbox-macos` (no `sandbox`
  feature; P7).

## Capabilities

### New Capabilities
- `pane-profiles`: where profiles live, file modes, names, strict parsing, paths and variables,
  read on every create, override rules, variables of a profile pane, refusal where a sandbox
  cannot be enforced, listing. ("A pane keeps its profile across a reload" is `bus-reload`'s;
  "tilr profiles" is `bus-tilr-client`'s.)

### Modified Capabilities
- `panes`: "Creating a pane", "The directory a pane starts in".

## Non-goals

- `tilr new --profile`, `tilr profiles` (`bus-tilr-client`); enforcing `[sandbox]`
  (`sandbox-linux`, `sandbox-macos`); profiles over `tilr.v1` (`CreatePane` is not changed, O2).

## Impact

`profile.rs`, `bus/core/{profiles,panes}.rs`, `config.rs`, tests, README. New optional
directory `profiles/` and key `profiles_dir`. Protocol: version 1; no `profiles` flag (P8).
Reload: until `bus-reload`, a pane keeps running its snapshot but `PaneInfo.profile` is empty
after a reload *(gap named by the split)*. Update from the previous release: no `profiles/`, no
change. Nothing released from `main` (O5).
