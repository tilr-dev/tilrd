# Proposal: The pane process title follows the pane

## Why

Open since the UUID work: the pane process's title (seen in the OS `ps`) should be updated live with its directory and command, and show the shell's pid (Basic Memory `main/projects/tilr/tilr-core-id-panelei-uuid-tilr-server-ps-i-peredacha-sostoianiia-pri-perezagruzke`).

## What Changes

- The pane process sets its title to `tilrd pane <id> <shell pid> <cwd> <command>`, updated on OSC 7 and foreground changes.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md`.

## Out of scope

- `tilrd ps` output (already has CWD and COMMAND).

## Impact

`crates/tilrd/src/pane.rs`. A reload execs with the original arguments, so the title must be set again after the exec. No protocol change.
