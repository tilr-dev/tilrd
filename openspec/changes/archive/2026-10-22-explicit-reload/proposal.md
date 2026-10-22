# Explicit reload after an update

## Why
Replacing the installed executable must not implicitly restart a running server. Installation and update must request reload explicitly while preserving panes and shells.

## What Changes
- Remove executable watching and the `--no-auto-reload` flag.
- Request reload explicitly after update or installation replaces the programs.
- Keep successor compatibility and resume checks, installed-path exec and live pane handover.
- Remove the superseded checked-file reload proposal.
- Keep completed installations on reload request failure, exit non-zero and report the failure consistently across installer, update and SSH deploy.

## Impact
- Reload, update, installation and CLI specifications.
- Supervisor startup, update, installer, SSH deploy and their integration tests.
