# Design: Sandboxed panes on macOS

Shared design: `openspec/design/unified-bus.md` §S10 (probe, accepted risk, where it is applied,
reload, profile generation, start-up check, cleanup), with §S6-S9 from `sandbox-linux`'s
`sandbox/mod.rs`. This change's part follows.

## Decisions

- **The `Platform` of `sandbox-linux`.** `macos.rs` implements the trait of `sandbox/mod.rs`;
  `apply_in_child` is not used on macOS (the child execs `sandbox-exec`, S10).
- **CI filter by name.** The job runs `cargo nextest run --features tilrd/failpoints -E
  'test(/^sandbox::/) | test(=reload::a_sandboxed_pane_stays_sandboxed_across_a_reload)'`;
  before `bus-reload` the second part matches nothing.
- **Shared requirements on macOS.** The requirements of `sandbox-linux` that also name macOS
  (feature check, Unix sockets, `sandbox-exec-missing`) are verified here by the same tests on
  the three runners.

## Reload and reconnect

S10 "Reload": unchanged pane argv; the sandbox is the kernel's; the trampoline path is used only
at spawn.

## Backpressure

None new.
