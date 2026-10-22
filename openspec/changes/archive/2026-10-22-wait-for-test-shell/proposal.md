## Why

The shared pane-opening helper returns after subscription without observing shell
startup. The archived reconnect-output-flake design records how PTY echo can
precede the first prompt, causing that prompt to prefix command output and prevent
an exact-row wait from matching.

## What Changes

- Wait for the first shell prompt before shared pane-opening helpers return.
- Share the controlled startup fixture and allow independently named FIFOs.
- Exercise delayed startup and the first command through the shared helper.

## Out of scope

- Production pane creation, protocol changes and custom-command pane fixtures.
- Reload state or update compatibility changes: only the test harness changes.

Protocol version remains unchanged; no feature flag is added.
