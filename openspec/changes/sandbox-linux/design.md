# Design: Sandboxed panes on Linux

Shared design: `openspec/design/unified-bus.md` Part S: §S1, §S2, §S5 (Landlock plus seccomp),
§S6, §S7, §S8, §S9. This change's part follows.

## Decisions

- **Neutral module first.** `sandbox/mod.rs` holds what both platforms share (S6-S9) behind
  `trait Platform { fn check() -> Result<Features, String>; fn prepare(&Snapshot, &PaneDirs) ->
  Result<Prepared, String>; fn apply_in_child(&Prepared) -> Result<(), Errno>; }`
  *(added by the split)*; `linux.rs` implements it; `sandbox-macos` adds `macos.rs`.
- **Failpoint for the profile test.** `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`
  (`pane-profiles`) switches to the failpoint `sandbox-unavailable` here.
- **macOS later.** Requirements that name macOS (feature check by `sandbox-exec`, Unix sockets
  on macOS, `sandbox-exec-missing`) are verified on macOS by `sandbox-macos`, which runs every
  `sandbox::*` test there.

## Reload and reconnect

The Landlock domain and seccomp filter stay on the shell and its descendants across the
pane process's exec (they are attached in the shell child, not in the pane process). `PaneInfo.
sandboxed` and the snapshot cross with `bus-reload`.

## Backpressure

None new.
