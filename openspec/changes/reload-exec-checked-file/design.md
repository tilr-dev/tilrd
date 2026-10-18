# Design

## Context

At 048b0e2 the supervisor runs its executable by path three times in one reload (`state-formats`, `crates/tilrd/src/supervisor.rs:1753`; the check child, `supervisor.rs:1784`; the exec, `supervisor.rs:1992-2000`) and each pane process twice (`crates/tilrd/src/pane.rs:546`, `pane.rs:564`). `crate::exe()` (`crates/tilrd/src/main.rs:215-221`) is `current_exe()` taken once at start: on Linux `/proc/self/exe`, on macOS `_NSGetExecutablePath`, i.e. the path the image was exec'd from.

## Decisions

### D1. Linux: one descriptor, exec'd through /proc/self/fd
Open `crate::exe()` `O_RDONLY | O_CLOEXEC` at the start of the reload; run `state-formats` and the check child as `/proc/self/fd/N` (the forked child still has the descriptor when it execs; close-on-exec closes it only after the kernel opened the image, which works for an ELF file) and exec the same path with `arg0` set to the installed path. Kept until the exec or the refusal, then closed.
- Rejected: `execveat(fd, "", AT_EMPTY_PATH)` / `fexecve`: std's `Command` has no such exec, and the check children are spawned through `Command`; `/proc/self/fd/N` is the same thing for both.
- Fallback where `/proc/self/fd` is missing (a chroot without procfs): the private copy of D2.

### D2. macOS: a private copy per process
No `fexecve`. Open the executable, `fstat` it (its identity, D4), clone it from the descriptor with `fclonefileat` into `<runtime dir>/exe/<pid>/<file name>` (a copy from the descriptor if cloning fails, e.g. another volume), directory 0700 and file 0700 in the private runtime directory; the copy is checked and exec'd. The file name is the installed one's (`tilrd`), so `ps -o comm` shows `tilrd`. The successor removes `exe/<own pid>/` once it runs (an unlinked image keeps running); a supervisor start sweeps `exe/<pid>/` directories of pids that are gone, as it sweeps state files.
- Each pane process makes its own copy: a clone is constant-time on APFS, and one shared copy would need the supervisor to keep it until every pane has exec'd, across its own exec. Cost on another volume: one copy of the executable per process for the length of the reload.
- Rejected: exec by path after checking that the identity is unchanged — still a window between the stat and the exec.

### D3. The installed path crosses the exec in TILR_RELOAD_EXE
The successor's `current_exe()` would be `/proc/self/fd/N`'s target (Linux; ` (deleted)` once replaced) or the copy (macOS); neither is what a later reload, the watch, the shims (`supervisor.rs:770`), new panes (`supervisor.rs:2936`) or server commands (`supervisor.rs:2205`) must use. The predecessor sets `TILR_RELOAD_EXE=<ino>:<size>:<mtime>:<mtime_nsec>:<path>` on the exec (and on the check children, so they see what the successor will). `main` reads it before any thread, removes it from the environment, and `exe()` returns the path; without it (a fresh start, or a predecessor from before this change) `exe()` is `current_exe()` as today. Not in the state file: that would change `supervisor-state` and `pane-state` and `state-formats` for a value the environment carries across `exec` anyway.

### D4. The watch starts from the file that runs
`watch_executable` (`crates/tilrd/src/reload.rs:63-81`) starts with `running = signature(path)` at the successor's start; if the path was replaced during the reload, that is the new file's signature and the replacement is never noticed. It starts instead from the identity passed in `TILR_RELOAD_EXE` (the file opened and exec'd), so a replacement during the reload triggers the next reload, which checks it.

### D5. What ps shows
argv is unchanged (argv[0] is the installed path, as `Command::new(exe)` gives today). Linux sets `comm` from the exec'd path's last component, which for `/proc/self/fd/7` is `7`: the successor (supervisor and pane) calls `prctl(PR_SET_NAME, <installed file name, 15 bytes>)` first thing in `main`, so `ps -o comm`, `pgrep -x tilrd` and `top` keep seeing `tilrd`. `/proc/<pid>/exe` names the installed file. macOS: `comm` is the copy's name (`tilrd`); `proc_pidpath` names the copy's path, which no longer exists after the successor's start.

## Reload and reconnect

This is the reload. The supervisor's and each pane's exec keep the pid, the arguments, the handed-over descriptors and the held signals as today; the descriptor of D1 is close-on-exec and not handed over. A refused reload closes the descriptor or removes the copy and carries on. A reload into an executable from before this change (a rollback) works, with one degradation on macOS: the old code ignores `TILR_RELOAD_EXE` and takes `current_exe()`. On Linux that is the installed file (or ` (deleted)` if it was replaced meanwhile, as today). On macOS it is the copy, which the old code does not remove: until the server is restarted its watch and its reloads look at the copy, not the installed path, so a later install is not picked up automatically (README says so; a restart, or `tilrd update` followed by a restart, recovers). Clients reconnect as on any reload.

## Backpressure

Not applicable: no client data.
