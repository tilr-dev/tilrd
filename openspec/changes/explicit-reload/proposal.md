# Explicit reload after an update

## Why
Replacing the installed executable must not implicitly restart a running server. Installation and update must request reload explicitly while preserving panes and shells.

## What Changes
- Remove executable watching and the `--no-auto-reload` flag.
- Request reload explicitly after update or installation replaces the programs.
- Keep successor compatibility and resume checks, installed-path exec and live pane handover.
- Remove the superseded checked-file reload proposal.

## Impact
- Reload, update and installation specifications.
- Supervisor startup, update, installer and their integration tests.
