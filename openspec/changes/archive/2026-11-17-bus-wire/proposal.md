# Proposal: The bus crate: wire, payload schemas and the Rust client

Зависит от: -

Стартует после: now; мержится после: -

Владеет: `crates/tilr-bus/**` (new crate; its `proto/` frozen after this change), the
workspace member line for it in `Cargo.toml`, the dependencies of `tilr-bus`.

## Why

Owner's decisions of 2026-11-17 (shared design `openspec/design/unified-bus.md`): one bus `tilr.bus.v1` for everything,
built in `main` beside `tilr.v1` as several changes, one PR each
(`openspec/design/unified-bus.md`, Owner's decisions, O1-O3). Every other change of the phase
codes against this crate, so it comes first, with every schema at once: parallel changes must
not edit `.proto` files (O4).

## What Changes

- New crate `crates/tilr-bus`: `proto/bus.proto` (package `tilr.bus.v1`, design B4), codec for
  WebSocket (flag byte, zstd at 256 B or more, no double compression, B3, B9) and stream sockets
  (u32 length), 16 MiB limit both ways, the error codes of B5.
- Every payload schema of every namespace, written now and frozen:
  `proto/tilr/{pane,server,fs,exec,clipboard,host,agent}/v1/*.proto` with the types moved
  (copied for now) from `tilr.proto` with their field numbers (T1), `PaneOutput{frame | images}`
  (B9), and the requests and replies of every method, event and stream of T2, T4-T9, P6, S9,
  R3-R5 and the `bus.*` control methods (B10), including fields used only by later changes
  (`PaneInfo.created_by`, `profile`, `sandboxed`; `bus.token {origin}`).
- The async Rust client (connect over fd, instance socket, WebSocket with access token or
  credential; `call`, `provide`, `stream` with credits, `emit`, `subscribe`, `activate`,
  `CredentialUpdate`), with a signature frozen after this change (O4). `Credential` is opaque
  bytes here; `Credential::attenuate` comes with `bus-rights`.
- The test runtime `crates/tilr-bus/examples/test_runtime.rs`, used by the tests of the later
  changes and by the SDKs' cross-language tests.

Nothing in `tilrd`, `tilr` or `tilr.proto` changes; no server serves the bus yet.

## Capabilities

None of its own (`skip_specs: true`): the codec unit tests named by the `component-bus` and
`protocol` requirements live here, but those requirements are added by the changes that serve
them (`bus-broker-core`, `bus-instances`, `remove-tilr-v1`).

## Non-goals

- Serving the bus (`bus-broker-core`), Biscuit (`bus-rights`), any change to `tilr.proto`.
- Generating SDK code from the schemas (`bus-sdk`).

## Impact

New crate only; `Cargo.lock`; `cargo deny` for its dependencies. Protocol: `tilr.bus.v1`
version 1 (B14), no feature flag; nothing crosses a reload; no effect on an update from the
previous release (nothing is released from `main`, O5).
