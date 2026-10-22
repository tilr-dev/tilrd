# Proposal: Verify update command waiting independently of first launch

## Why

A fresh executable can wait for operating-system launch policy before entering main. The integration harness starts its reload observation window before staged version checks finish, conflating launch latency with update completion. The client already waits for the update command without an execution deadline.

## What Changes

- Warm the exact server and client bytes used in the integration release before releasing the archive response and starting reload observation.
- Verify the existing client wait with a controlled peer and virtual time, independently of launch policy latency.
- Retain the real executable update, reload, reconnect and unchanged observation deadlines.

## Impact

Test harness and verification only; no product behavior or protocol changes.
