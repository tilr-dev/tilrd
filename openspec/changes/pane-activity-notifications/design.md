# Design

## Context

At 1163abd the pane process reads the pty and feeds the screen model (`crates/tilrd/src/pane.rs:346-358`, `on_output` at :417), which reports title and directory changes through libghostty-vt callbacks (`crates/tilr-screen/src/ghostty.rs:62-81`); libghostty-vt 0.2.2 also has `Terminal::on_bell`. Pane processes talk to the supervisor over a pipe of tagged packets (`crates/tilrd/src/link.rs:54-69`, kinds 0-5); the supervisor skips a kind it does not know (`supervisor.rs:3011`).

## Decisions

### D1. Bell
The screen model counts bells through `on_bell` (a bell, not a BEL ending an OSC string, which the VT parser consumes as a terminator); `Screen` gains `take_bells() -> u32`. After each `feed`, the pane process takes the count and, if it is not zero and its last report is a second old or more, sends one `PaneActivity{kind: BELL}`; later bells in that second are dropped (not deferred).

### D2. Command finished
The pane process remembers the shell's process group (the shell is a session leader, its pgid is its pid). After each read of pty output and on a 1 s tick it reads `tcgetpgrp` on the pty master: when it was another group and is now the shell's again, it sends `PaneActivity{kind: COMMAND_FINISHED}`. This needs a shell with job control (interactive shells put each command in its own group); a pane running a command directly, or a shell without job control, reports no `COMMAND_FINISHED`. A command that runs and ends between two checks with no output is missed (accepted: the shell's prompt is output, so a check follows almost every command).

### D3. Fan-out
The supervisor pushes `PaneActivity` to every greeted client on the interactive lane (small, rare); a client that does not know the message ignores it.

### D4. Pane to supervisor
A new link packet kind `ACTIVITY` (6) carries the encoded `PaneActivity` (`pane_id` filled in by the pane, `at_ms` the pane's wall clock in ms since the epoch). Rejected: fields in `PaneStatus` (a status is a state that is coalesced and compared, an activity is an event that must not be merged). Rejected: `RELAY` (that goes to the pane's subscribers only).

### D5. Wire shape
`message PaneActivity { string pane_id = 1; PaneActivityKind kind = 2; uint64 at_ms = 3; }`, `enum PaneActivityKind { PANE_ACTIVITY_KIND_UNSPECIFIED = 0; PANE_ACTIVITY_KIND_BELL = 1; PANE_ACTIVITY_KIND_COMMAND_FINISHED = 2; }`, a new `ServerMessage` variant. A client ignores a kind it does not know, so later kinds (output after silence) need no version bump.

## Reload and reconnect

Nothing is handed over (no pane-state or supervisor-state format change). After the exec a pane process reads `tcgetpgrp` once: if a command (another group) is in the foreground, its end is reported as usual. The bell's one-second window restarts. During the supervisor's exec, link packets wait in the pipe and are read by the new supervisor, which knows the kind; a pane that cannot hand over carries on with its old code and sends no `ACTIVITY`. Clients reconnect after a reload and get only later events. A client that reconnects for any reason gets no replay.

## Backpressure

One small message per bell second or per command per pane; on the interactive lane bounded like any push (a client whose queue fills is dropped as today, close 4001).
