# Proposal: A program that ignores SIGHUP does not outlive the server

## Why

- itests UC-086 is listed as a bug ("Сейчас баг — FAIL") without a scenario: when the server stops, a program in a pane that ignores SIGHUP is left running as an orphan (itests/USE_CASES.md group H). The `daemon-lifecycle` requirement "No shell outlives the server" states the intended behavior and has no test (its Notes).
- At 1163abd: a pane process that sees its supervisor pipe close returns at once (`crates/tilrd/src/pane.rs:381-383`, "Exiting closes the pty, which hangs up the shell"), with no SIGHUP-then-SIGKILL grace; only the pty hangup reaches the shell. `ClosePane` does have a grace (`pane.rs:603-606`, `KILL_GRACE` 2 s at `pane.rs:39-40`, SIGKILL at `pane.rs:390-393`), but it signals only the shell's process group (`signal_shell`, `pane.rs:467-471`), so a foreground job in a group of its own (any job of an interactive shell) that ignores SIGHUP survives a close too. `tilr kill-server` waits only for the server and pane process pids (`crates/tilr/src/daemon.rs:343-359`), so it reports success while such a program lives on.

## What Changes

- When a pane process loses its supervisor (server stop, `tilr kill-server`, or the supervisor killed), it notes the pty's foreground process group, sends SIGHUP to the shell's group and that foreground group, waits up to 2 s for the shell to exit and both groups to empty, then SIGKILLs what is left of the two groups, and only then exits (design D1, D2).
- `ClosePane` ends the same two groups the same way (today: the shell's group only).
- Background jobs in other process groups (`nohup cmd &`, `disown`ed jobs) are left as a terminal hangup leaves them (design D3).
- A reload is unaffected: the pane process execs and keeps the pty; nothing is signalled.
- itests UC-086 gets its scenario.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: "No shell outlives the server" says which processes are ended and how, with its tests.
- `panes`: "Closing a pane ends its program" also ends the foreground group.

## Non-goals

- Processes that left the pane's session (`setsid`, daemons started from the shell) or background jobs that ignore SIGHUP on purpose (`nohup`): they outlive the server as they outlive an SSH session or tmux.
- Execs (`exec` spec; change `exec-escaped-descendants`).
- A configurable grace period.

## Impact

`crates/tilrd/src/pane.rs` (stop path, `ClosePane`), `crates/tilrd/src/pty.rs` (foreground group of the master), `crates/tilrd/tests/panes.rs`, itests `uc_086.py` (new), USE_CASES.md, itests/USE_CASES.md (UC-086), README "Stopping". A server stop takes up to 2 s longer when a program ignores SIGHUP (`tilr kill-server` waits 10 s). No protocol change, no PROTOCOL_VERSION or feature change, no state format change; nothing crosses a reload (a reload signals nothing). An update from the previous release changes nothing until the server stops. Not breaking.

## Decision (2026-10-18)

Confirmed as designed: on stop, SIGHUP then (after 2 s) SIGKILL to the shell's process group and the foreground group. `nohup`'d and background jobs survive, as with `tmux kill-server` and an ssh disconnect. Rejected: systemd `KillMode=control-group` style (kill everything of the pane), which breaks `nohup` expectations and is not portable to macOS.
