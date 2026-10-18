# Design

## Decisions

### D1. Bell
The pane process counts BEL (0x07) as the screen model reports it (a bell, not a BEL ending an OSC string) and sends `PaneActivity{kind: BELL}` to the supervisor over its link, at most one per second per pane; later bells in that second are dropped.

### D2. Command finished
The pane process remembers the shell's process group (the shell is a session leader, its pgid is its pid). After each read of pty output and on a 1 s tick it reads `tcgetpgrp` on the pty: when it was another group and is now the shell's again, it sends `PaneActivity{kind: COMMAND_FINISHED}`. This needs a shell with job control (interactive shells put each command in its own group); a pane running a command directly, or a shell without job control, reports no `COMMAND_FINISHED`. A command that runs and ends between two checks with no output is missed (accepted: the shell's prompt is output, so a check follows almost every command).

### D3. Fan-out
The supervisor pushes `PaneActivity` to every greeted client on the interactive lane (small, rare); a client that does not know the message ignores it.

## Reload and reconnect

Nothing is handed over. After the exec a pane process reads `tcgetpgrp` once: if a command is in the foreground, its end is reported as usual. Clients reconnect after a reload and get only later events. A client that reconnects for any reason gets no replay.

## Backpressure

One small message per bell second or per command per pane; on the interactive lane bounded like any push (a client whose queue fills is dropped as today).
