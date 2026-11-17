# Design: Launch profiles for panes

Shared design: `openspec/design/unified-bus.md` Part P (P1-P8) and §S1 (the sandbox is asked
for only by naming a profile). This change's part follows.

## Decisions

- **Snapshot type now, carried later.** `profile.rs` defines the resolved snapshot (argv, cwd,
  env, sandbox) that the pane keeps; `bus-reload` writes it into `PaneState` (supervisor-state 3).
- **Sandboxed profiles refused.** Without the `sandbox` feature every profile with `[sandbox]`
  is refused with P7's text; `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`
  needs no failpoint here, and `sandbox-linux` switches it to the failpoint
  `sandbox-unavailable`.
- **`pane.profiles` in the method list.** This change adds `pane.profiles` to
  `bus_terminal::tilrd_provides_the_terminal_methods`.

## Reload and reconnect

Profiles are read on create only; nothing is cached. The snapshot crosses a reload with
`bus-reload`.

## Backpressure

None new: at most 1000 profiles listed, texts cut at 1 KiB (P6).
