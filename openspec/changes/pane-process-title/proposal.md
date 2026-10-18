# Proposal: The pane process title follows the pane

## Why

Open since the UUID work: the pane process's title (seen in the OS `ps`) should be updated live with its directory and command, and show the shell's pid (Basic Memory `main/projects/tilr/tilr-core-id-panelei-uuid-tilr-server-ps-i-peredacha-sostoianiia-pri-perezagruzke`).

Checked at 1163abd: still not done. A pane process shows only the arguments it was started with (`tilrd pane --id <id> [--cols N --rows N] [--cwd DIR] [-- COMMAND]`, `crates/tilrd/src/pane.rs:49-63`); nothing in `crates/` sets a process title (no `prctl`, no argv rewrite). The pane process knows the directory (OSC 7, `PaneStatus.cwd`, `pane.rs:667-675`, `crates/tilr-screen/src/ghostty.rs:443-450`) but not the foreground command. The pane process reads its own arguments again for a reload (`std::env::args_os()` at `pane.rs:548` and `pane.rs:565`); on Linux and macOS `args_os` reads the live argv memory, so rewriting argv in place without keeping a copy would break the reload.

## What Changes

- The pane process's visible command line (`ps -o args`, `/proc/<pid>/cmdline`) becomes its start arguments followed by ` [shell <shell pid> in <cwd>]`, or ` [shell <shell pid> in <cwd>: <foreground command>]` while a command other than the shell is in the foreground of the pty.
- It is updated when the directory changes (OSC 7) and when the pty's foreground process group changes (checked after pty output and on a 1 s tick), and set again by the successor after a reload.
- The arguments a pane process execs its successor with (and checks it with) are taken once at start, before the title is set, instead of from `std::env::args_os()` at reload time.
- The title uses the argv area plus the environment area after it (the environment strings are copied first), as PostgreSQL and nginx do; it is cut at the end of that area. Where that cannot be set up, the title stays the start arguments and the server logs one warning.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md` ("The pane process shows which pane it is" modified; "The pane process title follows the pane" added).

## Non-goals

- `tilrd ps` / `tilr ps` output (already has CWD and COMMAND from `PaneInfo`).
- A foreground command in `PaneInfo` or any protocol message (no protocol change, no PROTOCOL_VERSION bump, no feature).
- The supervisor's title.
- Setting `comm` (`prctl(PR_SET_NAME)`, 15 bytes): it stays `tilrd`.

## Impact

`crates/tilrd/src/pane.rs` (keep the start arguments, title updates, foreground group check), a new `crates/tilrd/src/title.rs` (argv/environment area, Linux and macOS), `crates/tilrd/tests/panes.rs` and `reload.rs`, README "Telling panes apart", USE_CASES.md. Not breaking: anything that matches ` pane --id <id>` in the command line (itests `tt`: `" pane --id "`, `tilrd.* pane --id `) still matches, since the title starts with the start arguments. Reload: nothing new crosses it (no pane state format change); the successor is exec'd with the kept start arguments and sets the title again from its own reads. Update from the previous release: the old pane process execs the new one with its (unchanged) start arguments, so it works.
