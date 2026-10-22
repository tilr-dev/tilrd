# Bound input delivery history

## Why
Each client process creates a fresh identity, but completed sessions leave delivery counters in every pane indefinitely, including across exec.

## What Changes
- Finalize an identity after a normal client exit has acknowledged all admitted input.
- Retain disconnected identities for one hour without reconnect, then remove their delivery history.
- Start delivery history empty after exec while preserving queued PTY bytes and latency marks.
- Document accepted replay duplication after expiry or reload. Keep protocol version 1.

## Capabilities
- protocol: input delivery lifetime and finalization.
- client-liveness: bounded reconnect deduplication.
- reload: delivery history resets at exec.
