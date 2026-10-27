# Design

## D1. Crate boundary
`tilr-sync` owns the sync protocol, the index (per root: files, version vectors, block hashes, a local sequence), block transfer, conflict copies and name rules, behind an API that takes: a QUIC endpoint (or a socket to build one), a list of roots, a store directory, and the set of paired peer keys. It knows nothing of tilrd's protocol, panes or tokens. The Mac/iPad apps embed the same crate (via their FFI) as the other side.

## D2. Where it runs in tilrd
In the supervisor process, on its own tasks (open question: a separate sync child process like pane processes, if its CPU or memory use disturbs routing). Watches use the platform watcher (inotify / FSEvents); roots are `[files] roots`, the read-only setting applies.

## D3. Pairing
Over an authenticated tilrd connection (access token or unix socket), the client sends `SyncPair { device_id, public_key, name }`; the server stores the key in the state directory and answers `SyncPaired { server_key, port, roots }`. The QUIC handshake then authenticates both sides by these raw public keys; a peer whose key was not paired is refused at the handshake. Unpairing removes the key (`SyncUnpair`). No pairing over QUIC itself.

## D4. Endpoint across a reload
The UDP socket is bound once and its descriptor is handed over in the supervisor state like the TCP listener (supervisor-state format bump; a successor that does not read it is not reloaded into, per `state-formats`). QUIC connections end at the exec; peers reconnect and catch up by index (sequence numbers), so nothing else is handed over. The index and keys are in the state directory, written whole (temp file, sync, rename).

## D5. Data and backpressure
File data goes over QUIC streams, not the WebSocket; QUIC flow control bounds what is in flight per stream and connection; the engine holds at most a fixed number of blocks in memory per peer.

## Relationship with quic-transport
Independent. Both may later share one UDP port by ALPN; not required by either.

## Open questions
- iroh (NAT traversal, relays) or plain quinn (LAN plus Cloud tunnel)?
- Supervisor tasks or a sync child process (D2)?
