# Hybrid file watching on macOS

## Why

Native tracing on 2026-10-27 recorded filesystem changes reaching fseventsd processing 20–75 seconds later, including a native client with latency zero and NoDefer. The existing daemon watcher and app watcher both depend on that service. The owner requested a hybrid investigation and rejected keeping every file open (research brief, 2026-10-27). Existing batching is about 100 ms in the daemon and 200 ms by default in the app, independently of native delivery.

## What Changes

This research proposes bounded directory vnode watches plus FSEvents and incremental metadata reconciliation. It compares established implementations, quantifies work rather than inventing CPU measurements, and exposes the unresolved latency tradeoff in design.md. The files delta describes proposed additive recovery behavior only; it is not implemented.

## Capabilities

- `files`: additive recovery from missing or coarse filesystem notifications.
- App tree watching is analyzed here; its eventual site-serving delta belongs to its implementation change after owner selection.

## Non-goals

- Product code, dependencies, configuration defaults, wire format or protocol-version changes.
- Changing current batching, the directory-sync change, or existing requirements.
- Per-file descriptors, replacing native directories with a virtual filesystem, or promising hard real-time scheduling.

## Impact

The proposed implementation affects daemon watch registration/reporting and app FolderWatcher consumers. No breaking change, migration, new feature flag or state-format change is introduced by this research. Protocol version remains 1. Native registrations and scan state would be rebuilt after reload; clients reconnect and register again, as today. Implementation work must preserve previous-release update behavior and connection-owned watch cleanup. This research change stays open for implementation planning.
