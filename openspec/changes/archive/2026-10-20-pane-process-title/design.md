# Design

## Context

At 1163abd a pane process is `tilrd pane --id <id> ...` for its whole life (`crates/tilrd/src/pane.rs:49-63`), and both the reload check and the exec of the successor read `std::env::args_os()` again (`pane.rs:548`, `pane.rs:565`). Rust's `args_os` on Linux (glibc) and macOS builds the list from the process's argv pointers each time it is called, so it would return a rewritten title. The pane process is single-threaded until `pane::run` builds its runtime (`pane.rs:169-179`).

## Decisions

### D1. Title format: the start arguments, then what the pane is doing
`<start arguments> [shell <pid> in <cwd>]` while the shell is in the foreground, `<start arguments> [shell <pid> in <cwd>: <command>]` otherwise. The start arguments stay first so that `ps`, `pgrep -f " pane --id <id>"` and the itests helpers keep finding pane processes by id; the bracket is what changes. `<cwd>` is `PaneInfo.cwd` (OSC 7; before the shell reports one, the start directory); `<command>` is the foreground group leader's command line, NULs turned to spaces, at most 80 bytes. When the command line cannot be read or is empty (another user's process such as `sudo vim`: macOS refuses `KERN_PROCARGS2`; a zombie leader), the leader's process name is shown (`/proc/<pid>/comm`, macOS `proc_pidinfo` short BSD info); when the leader is gone (`cat file | less` after `cat` is reaped), `?`. The idle form would claim the shell is in the foreground. Rejected: replacing the arguments entirely with `tilrd pane <id> <pid> <cwd> <command>` (the first draft): it hides `--cols`/`--cwd`, which `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` and people reading `ps` rely on, and breaks matching on ` pane --id `.

### D2. Where the title is written
At the start of `pane::run`, before the runtime and any thread: copy the start arguments into a `Vec<OsString>` (used for the reload check and the exec), copy every environment string to the heap and point `environ` at the copies, then treat the argv strings plus the environment strings after them (contiguous in memory on Linux and macOS) as the title area. Writing a title fills the area from argv[0] and pads it with NULs; a title longer than the area is cut. Linux reads a command line past the argv area when its last byte is not NUL (kernel `get_mm_cmdline`), and macOS `ps` reads the same memory through `KERN_PROCARGS2`. The reusable area ends at the first noncontiguous environment entry (checked by pointer equality); a moved entry does not discard the contiguous prefix. All entries, including those beyond the gap, are copied; if even that is too small for the start arguments plus ` [`, the title is not changed and one warning is logged (`pane title: no room`). Rejected: `prctl(PR_SET_MM_ARG_START)` (needs `CAP_SYS_RESOURCE`); `PR_SET_NAME` (15 bytes, `comm` only).

### D3. Foreground command
After each batch of pty output and on a 1 s tick, the pane process reads `tcgetpgrp` on the pty master. When the group changes and unconditionally on each tick, it reads the leader's command line (Linux `/proc/<pgid>/cmdline`; macOS `sysctl` `KERN_PROCARGS2`); the shell's own group means no command. The pty-output path caches the command per group; the tick refreshes the leader command even if the group is unchanged, so a silent exec cannot leave a stale title. The title is rewritten only when its bytes change. An idle pane costs one `tcgetpgrp` a second; a foreground command adds one cmdline read per second. A command that starts and ends between two checks is not shown (it is gone anyway). Change `pane-activity-notifications` watches the same `tcgetpgrp` (its D2); whichever lands second reuses the other's check instead of adding a second tick.

## Reload and reconnect

The pane process execs its successor with the start arguments kept in D2 (not the title), so the successor parses the same `--id`/`--cols`/`--cwd`/command as today. The successor sets the title itself once it has resumed: the directory comes from the restored screen (`pwd` is in the snapshot) and the foreground group from `tcgetpgrp`. Nothing is handed over; no pane state format change. A client reconnect does not touch the title.

## Backpressure

Not applicable: no data goes to clients.

## Risks

- Repointing `environ` must happen before any thread or code retaining borrowed C environment pointers exists; earlier Rust environment reads own their values. `pane::run` is entered from `main` before the runtime is built, and the change asserts the order with a comment and a debug check that the process has one thread (`/proc/self/task` on Linux). Runtime argv pointers are also moved to heap copies, so readers retain the original arguments while the kernel reads the original string area for `ps`.
- The shell is spawned with an environment built from `std::env::vars()` (reads `environ`), which then sees the copies: unchanged values.

## Supervisor follow-up: established causes (2026-10-20)

The old per-pgid cache retained the first argv observed for a foreground leader, including the pre-exec shell during the tcsetpgrp/exec window. A two-FIFO wrapper and the `pane-title-read-command` pause hook guarantee that the wrapper command is cached before a controlled silent exec to sleep. With the tick invalidation removed, the regression test times out with `sh ./title-wrapper` still in the title after exec. The fix refreshes cmdline on every tick.

The old environment loop discarded every env byte when any entry was noncontiguous. The synthetic moved-entry test returns an 11-byte argv-only area before the fix instead of the 21-byte argv plus HOME prefix. Separate extension and copying loops retain that prefix, stop permanently at the gap, and preserve all original entries and arguments.
