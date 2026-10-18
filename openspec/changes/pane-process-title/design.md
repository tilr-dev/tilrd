# Design

## Context

At 1163abd a pane process is `tilrd pane --id <id> ...` for its whole life (`crates/tilrd/src/pane.rs:49-63`), and both the reload check and the exec of the successor read `std::env::args_os()` again (`pane.rs:548`, `pane.rs:565`). Rust's `args_os` on Linux (glibc) and macOS builds the list from the process's argv pointers each time it is called, so it would return a rewritten title. The pane process is single-threaded until `pane::run` builds its runtime (`pane.rs:169-179`).

## Decisions

### D1. Title format: the start arguments, then what the pane is doing
`<start arguments> [shell <pid> in <cwd>]` while the shell is in the foreground, `<start arguments> [shell <pid> in <cwd>: <command>]` otherwise. The start arguments stay first so that `ps`, `pgrep -f " pane --id <id>"` and the itests helpers keep finding pane processes by id; the bracket is what changes. `<cwd>` is `PaneInfo.cwd` (OSC 7; before the shell reports one, the start directory); `<command>` is the foreground group leader's command line, NULs turned to spaces, at most 80 bytes. Rejected: replacing the arguments entirely with `tilrd pane <id> <pid> <cwd> <command>` (the first draft): it hides `--cols`/`--cwd`, which `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` and people reading `ps` rely on, and breaks matching on ` pane --id `.

### D2. Where the title is written
At the start of `pane::run`, before the runtime and any thread: copy the start arguments into a `Vec<OsString>` (used for the reload check and the exec), copy every environment string to the heap and point `environ` at the copies, then treat the argv strings plus the environment strings after them (contiguous in memory on Linux and macOS) as the title area. Writing a title fills the area from argv[0] and pads it with NULs; a title longer than the area is cut. Linux reads a command line past the argv area when its last byte is not NUL (kernel `get_mm_cmdline`), and macOS `ps` reads the same memory through `KERN_PROCARGS2`. If the areas are not contiguous (checked by pointer arithmetic), only the argv area is used; if even that is too small for the start arguments plus ` [`, the title is not changed and one warning is logged (`pane title: no room`). Rejected: `prctl(PR_SET_MM_ARG_START)` (needs `CAP_SYS_RESOURCE`); `PR_SET_NAME` (15 bytes, `comm` only).

### D3. Foreground command
After each batch of pty output and on a 1 s tick, the pane process reads `tcgetpgrp` on the pty master. When the group changes, it reads the leader's command line (Linux `/proc/<pgid>/cmdline`; macOS `sysctl` `KERN_PROCARGS2`); the shell's own group means no command. The title is rewritten only when the group or the directory changed, so an idle pane costs one `tcgetpgrp` a second. A command that starts and ends between two checks is not shown (it is gone anyway). Change `pane-activity-notifications` watches the same `tcgetpgrp` (its D2); whichever lands second reuses the other's check instead of adding a second tick.

## Reload and reconnect

The pane process execs its successor with the start arguments kept in D2 (not the title), so the successor parses the same `--id`/`--cols`/`--cwd`/command as today. The successor sets the title itself once it has resumed: the directory comes from the restored screen (`pwd` is in the snapshot) and the foreground group from `tcgetpgrp`. Nothing is handed over; no pane state format change. A client reconnect does not touch the title.

## Backpressure

Not applicable: no data goes to clients.

## Risks

- Repointing `environ` must happen before any thread or `getenv` caller exists; `pane::run` is entered from `main` before the runtime is built, and the change asserts the order with a comment and a debug check that the process has one thread (`/proc/self/task` on Linux).
- The shell is spawned with an environment built from `std::env::vars()` (reads `environ`), which then sees the copies: unchanged values.
