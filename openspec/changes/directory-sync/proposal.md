# Proposal: Directory synchronization (group S) in a `tilr-sync` crate embedded in tilrd

## Why

itests group S (rsync-like directory sync) was deferred by Dan on 2026-10-16 ("пока не делаем, вернуться позже"); the open question was whether sync is part of the tilrd protocol over WebSocket or a separate core on QUIC. The research note decided on 2026-10-07 for an own push protocol with ≤10 ms latency in a LAN, catch-up by index in the spirit of Syncthing BEP, with an embedded Rust core on iroh/quinn (itests/USE_CASES.md "Отложено"; Basic Memory `main/projects/tilr/tilr-zhivaia-sinkhronizatsiia-mac-i-pad-protokoly-syncthing-bep-push-za-10-ms`).

## Decision (2026-10-18)

- A separate crate `tilr-sync`, the same core the Mac/iPad apps embed: own push protocol (≤10 ms in a LAN), catch-up by index (version vectors, block hashes, sequence numbers), QUIC via iroh or quinn, name rules (case collisions, NFC/NFD).
- tilrd embeds it: tilrd opens the QUIC endpoint; pairing and key exchange go over the already authenticated tilrd WebSocket (token or unix socket); sync roots are the `[files]` roots.
- Independent of `quic-transport`: sync has its own QUIC endpoint; terminal-over-QUIC is a separate change.
- Rejected: a separate sync daemon (a second service to install, update and authenticate).
- Rejected: sync over the WebSocket bulk lane (one TCP stream, head-of-line blocking, no ≤10 ms push under load).

## What Changes

- New workspace crate `crates/tilr-sync` (engine: index, block transfer, conflicts, name rules; no tilrd types).
- tilrd: a UDP socket and QUIC endpoint for sync, handed over across a reload like the TCP listener; config `[sync]` (enabled, port); feature `sync` in `HelloAck.features` (it can be switched off).
- Protocol: `SyncPair` / `SyncPaired` messages over the WebSocket to exchange device keys and the endpoint address (design D3).
- Cases to cover (itests group S): first sync of empty/non-empty trees and merge, edits propagating (≤10 ms target in a LAN), catch-up by index after a cut, conflicts kept as copies, deletes and renames not resurrected, block-level transfer of small edits, `A.txt`/`a.txt` and NFD/NFC across Linux and macOS, symlinks and permissions, inotify `max_user_watches`, no partial file at the receiver.

## Capabilities

### New Capabilities
- `directory-sync`: see `specs/directory-sync/spec.md`.

## Non-goals

- Changes to the existing `Fs*` requests.
- Syncthing interoperability; Syncthing as the engine.
- Terminal traffic over QUIC (`quic-transport`).
- Sync between two tilrd servers without a client (later, on the same core).

## Impact

New crate and dependencies (`cargo deny` review of quinn/iroh), `crates/tilrd` (endpoint, config, pairing), `tilr.proto` (additive messages; per the protocol version policy in `openspec/config.yaml`, version 1 without a bump before the first public release; `sync` is a feature because it can be off), supervisor state (the UDP socket's descriptor: supervisor-state format bump). The index and paired keys live in the state directory, so they survive reloads and restarts. Not breaking.
