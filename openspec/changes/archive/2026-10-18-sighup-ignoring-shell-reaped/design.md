# Design

## Context

A pane's shell is a session leader on the pty (`pty.rs:226`); an interactive shell puts each job in a process group of its own. When the pty master closes, the kernel sends SIGHUP to the session's foreground process group and the session leader; a shell forwards it to its jobs. A program that ignores SIGHUP survives that. At 1163abd the pane process exits right after its supervisor pipe closes (`pane.rs:381-383`) and the only grace is on `ClosePane`, aimed at the shell's group alone. Found while implementing: on a clean stop the pane process never got that far either, since the supervisor spawns it with `kill_on_drop(true)` (`supervisor.rs`, `start_pane_process`) and its handle is dropped when the supervisor returns, so every pane process was SIGKILLed as the server exited.

## Decisions

### D1. Two groups: the shell's and the foreground one
The pane process ends the shell's process group (its pid) and the pty's foreground process group (`tcgetpgrp` on the master, read before anything is signalled). These are exactly what a terminal hangup is aimed at, so the server's stop ends what a closed terminal would have ended and no more.

### D2. SIGHUP, 2 s, SIGKILL, then exit
On losing its supervisor the pane process: reads the foreground group; sends SIGHUP to both groups; waits up to `KILL_GRACE` (2 s) until the shell is reaped and `kill(-pgid, 0)` fails with ESRCH for both; SIGKILLs both groups if not; sends its last frame as today; exits. `ClosePane` arms the same sequence (the existing `kill_at` deadline, now for both groups). A pgid is reused only after its group has emptied; the SIGKILL goes out at most 2 s after the check, so hitting a stranger needs a pid reused as a group id within that window: accepted.

### D3. Background jobs are left
A background job of another process group that ignores SIGHUP (`nohup cmd &`) is what a user asks to outlive a hangup; tmux and SSH leave it, and so does this change. Scanning the whole session (Linux `/proc`, macOS `proc_listallpids`) was rejected: it would kill such jobs and needs per-platform process listing.

### D4. The stopping server waits for its pane processes
After its event loop ends the supervisor closes the pipe to each pane process (dropping the queue's sender and the pipe its writer hands back) and waits up to `PANE_STOP` (3 s: the 2 s grace and a margin) for each reader task, which ends once its process has exited and been reaped; a pane process still there after that is SIGKILLed when its handle drops, as before. So `tilr kill-server`, which waits for the server's pid, returns only once the pane programs are gone, and a server stop takes up to 2 s longer only when something ignores SIGHUP.

## Reload and reconnect

Unchanged: a reload makes the pane process exec in place with the pty and the shell; nothing is signalled and the stop path is not taken. The check child of a reload (archived `reload-rollback`) runs no shell and never takes this path. Clients see `PaneExited` as today, at most 2 s later than before when something ignores SIGHUP.

## Backpressure

Not applicable.
