# Clear a pane's screen and history

## Why

Clients need to clear the server-owned terminal screen and optionally its scrollback so that clearing survives reconnecting and subsequent full frames.

## What Changes

- Add `ClearPane{pane_id, history}` to the client protocol.
- Clear the emulator directly, return the view to the live screen and send a full frame to every viewer without writing to the pty.
- Document and test clearing with and without history, reconnecting, continued output and unknown panes.

## Impact

The protocol remains version 1. Older servers ignore unknown messages. Client binding generation belongs to the consuming client repository.
