# Proposal: Directory synchronization (group S)

Блокер: Where should directory sync live? Options: (a) in tilrd's protocol over the existing WebSocket (bulk lane, files roots, one connection); (b) a separate sync core on QUIC (iroh/quinn), tilrd only starts it; (c) Syncthing as the engine, tilrd configures it. Recommendation: (b) for the ≤10 ms target and independent congestion control, with tilrd starting and authenticating it; revisit after `quic-transport`.

## Why

itests group S (rsync-like directory sync) was deferred by Dan on 2026-10-16 ("пока не делаем, вернуться позже"); the open question is whether sync is part of the tilrd protocol over WebSocket or a separate core on QUIC. The research note decided on 2026-10-07 for an own push protocol with ≤10 ms latency in a LAN and Syncthing as a fallback, with an embedded Rust core on iroh/quinn as an option (itests/USE_CASES.md "Отложено"; Basic Memory `main/projects/tilr/tilr-zhivaia-sinkhronizatsiia-mac-i-pad-protokoly-syncthing-bep-push-za-10-ms`).

## What Changes

- To be designed. Cases already listed: first sync of empty/non-empty trees and merge, edits propagating (≤10 ms target in a LAN), catch-up by index after a cut, conflicts kept as copies, deletes and renames not resurrected, block-level transfer of small edits, `A.txt`/`a.txt` and NFD/NFC across Linux and macOS, symlinks and permissions, inotify `max_user_watches`, no partial file at the receiver.

## Capabilities

### New Capabilities
- `directory-sync`: to be specified once decided.

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Changes to the existing `Fs*` requests.

## Impact

A new capability `directory-sync`. If in tilrd: new proto messages (feature flag `sync`), state across reloads (watch sets, indexes), bulk lane for data. If a separate core: no tilrd change beyond maybe launching it.
