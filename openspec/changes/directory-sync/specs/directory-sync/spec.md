## ADDED Requirements

### Requirement: Sync is offered unless turned off
The server SHALL list `"sync"` in `HelloAck.features` and listen for sync peers on its QUIC endpoint unless `[sync] enabled = false`; with sync off it MUST NOT open the endpoint and MUST answer `SyncPair` with an error.

#### Scenario: Sync turned off
- **WHEN** the server runs with `[sync] enabled = false` and a client sends `SyncPair`
- **THEN** `"sync"` is not in `HelloAck.features`, the answer is an error, and no UDP port is open

### Requirement: Pairing goes over an authenticated connection
A sync peer SHALL be paired only by `SyncPair` on a greeted tilrd connection; the server SHALL answer `SyncPaired` with its key, port and roots, and its QUIC endpoint MUST refuse a peer whose key was not paired.

#### Scenario: Unpaired peer
- **WHEN** a peer with an unknown key connects to the sync endpoint
- **THEN** the handshake is refused and nothing is synced

#### Scenario: Paired peer
- **WHEN** a client pairs with `SyncPair` and connects with that key
- **THEN** the handshake succeeds and the peer sees the server's roots

### Requirement: Only the shared directories are synced
The roots a peer can sync SHALL be the `[files] roots`; with the files read-only, the server MUST NOT change files on a peer's behalf.

#### Scenario: Path outside the roots
- **WHEN** a peer offers a change to a path outside every root
- **THEN** the server does not apply it

### Requirement: The sync endpoint survives a reload
A reload SHALL keep the sync UDP socket (same port); peers SHALL reconnect and catch up by index, with no file lost or duplicated.

#### Scenario: Edit during a reload
- **WHEN** a file is changed on the peer while the server reloads
- **THEN** after the reload the server has the change and the port is the same
