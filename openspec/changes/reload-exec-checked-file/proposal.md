# Proposal: a reload execs the very file it checked

## Why

Itsaplan TILR-108 (phase 5, tech debt). The reload check (archived change `2026-10-18-reload-rollback`) runs the new executable before betting the panes on it, but by path, and the exec then opens the path again. Checked at 048b0e2:

- Supervisor: `crate::exe()` (`crates/tilrd/src/supervisor.rs:1751`) is run as `tilrd state-formats` (`supervisor.rs:1753`, `reload::check_executable`, `crates/tilrd/src/reload.rs:145-161`, `Command::new(exe)`), then as the check child with `TILR_RELOAD_CHECK` (`supervisor.rs:1784-1789`, `reload::check_successor`, `reload.rs:167-189`, `Command::new(exe)`), then exec'd by path (`supervisor.rs:1992-2000`, `Command::new(exe).args(...).exec()`), with the clients served meanwhile (`while_serving`) and the panes detached and a `reload-before-exec` point between (`supervisor.rs:1869`). Each pane process does the same on its own: check child by path (`crates/tilrd/src/pane.rs:517`, `pane.rs:546-551`), then `Command::new(exe).exec()` (`pane.rs:564-566`).
- So a file replaced between the check and the exec (an install or build finishing during the up to 10 s check, `CHECK_TIMEOUT`, `reload.rs:48`) is exec'd unchecked: a successor that cannot resume ends the server and its shells, the outcome the check was added to prevent. The existing test of a second replacement during a reload (`reload::an_executable_replaced_again_during_a_reload_costs_no_shell`; requirement "A second replacement during a reload costs no shell") only asks that the server stays up "whatever it makes of the second file", with a second file that resumes.
- `crate::exe()` is the path resolved once at start (`crates/tilrd/src/main.rs:215-221`; its comment: on Linux the answer goes stale as soon as the file is replaced). It is also what the watch compares (`supervisor.rs:515`), what the clipboard and `tilr` shims link to (`supervisor.rs:770`), what new panes are started from (`supervisor.rs:2936`) and what server commands run (`supervisor.rs:2205`); the exec leaves argv[0] the path string (`Command::new(exe)`).

## What Changes

- At the start of a reload the supervisor opens its executable once, and every step uses that open file: `state-formats`, the check child and the exec. Linux: the file descriptor (O_CLOEXEC), run and exec'd as `/proc/self/fd/N`; where `/proc` is not mounted, as macOS does. macOS: a private copy made from the descriptor (`fclonefileat`, else a copy) to `<runtime dir>/exe/<pid>/<file name>` (directory 0700, file 0700), checked and exec'd; the successor removes it once it runs, and a start sweeps copies of pids that are gone.
- Each pane process does the same for its own check and exec.
- The successor knows the path it was installed at, not the descriptor or copy it was exec'd from: the predecessor passes it (with the identity of the file it opened: inode, size, mtime) in `TILR_RELOAD_EXE`, read once at start and removed from the environment before any thread or child, so panes, execs and shims see no such variable. `crate::exe()` is that path; a later reload, the watch, the shims, new panes and server commands use it, so a later reload re-reads the path. The watch starts from the identity of the file that was exec'd, so a file replaced during the reload is noticed and checked by the next one.
- What `ps` shows is unchanged: argv[0] stays the installed path (`arg0`), the arguments are the same; on Linux the successor sets its name (`comm`, which an exec through `/proc/self/fd/N` makes the descriptor number) back to the installed file's name with `PR_SET_NAME`. `/proc/<pid>/exe` (Linux) names the installed file (with ` (deleted)` once it is replaced, as today); on macOS the process's executable path is the private copy, gone once the successor removed it.

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md` (new requirement: the executable checked is the one exec'd; a second replacement during a reload is not exec'd unchecked).

## Non-goals

- Verifying signatures or checksums of the executable (`tilrd update` checks the archive's `.sha256`, `update` spec).
- Noticing a replacement during the check sooner than the watch does after the exec.
- Changing what the check runs or answers (`TILR_RELOAD_CHECK`, `reload-check 1`).

## Impact

`crates/tilrd/src/reload.rs` (open-once executable: descriptor or private copy; `check_executable` / `check_successor` take it; `TILR_RELOAD_EXE`; sweep of copies), `crates/tilrd/src/main.rs` (`exe()` from `TILR_RELOAD_EXE`; `PR_SET_NAME` on Linux), `crates/tilrd/src/supervisor.rs` (reload and `exec_successor`), `crates/tilrd/src/pane.rs` (pane reload; a failpoint between check and exec), `crates/tilrd/tests/reload.rs`, USE_CASES.md, README "Reloading without closing shells". Survives a reload by construction (`TILR_RELOAD_EXE` is how the path crosses it); no state file and no `state-formats` change. Update from the previous release: an old server reloads into the new executable by path as today, and the new one, finding no `TILR_RELOAD_EXE`, falls back to `current_exe()` as today; from then on reloads exec the checked file. A rollback to a version from before this change works, but on macOS that version then runs from the private copy and watches it, not the installed path, until the server is restarted (design, "Reload and reconnect"). No protocol change; not breaking.
