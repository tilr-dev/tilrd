## Why

The reload storm test can mistake a successor's initialization for completion of a queued reload. Its verification client then loses its connection to the next reload; test cleanup sends SIGTERM, making the failure look like a server shutdown bug.

## What Changes

- Exercise a burst of signals deterministically while the successor is restoring its panes.
- Wait for the reload queued by those signals, including its successor's resume, before checking the shell.

## Impact

- A test-only pause at the end of pane restoration and the reload integration test.
- No change to normal signal handling.
