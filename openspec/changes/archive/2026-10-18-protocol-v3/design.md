# Design

## Context

At 1163abd `PROTOCOL_VERSION` is 2 (`crates/tilr-proto/src/lib.rs:14`) and `HelloAck.features` is `clipboard`, `ping`, `files`, `host-events`, `agent`, `exec` (`crates/tilrd/src/supervisor.rs:2051-2058`). `clipboard`, `files`, `host-events`, `agent` and `exec` can be off (`--no-clipboard`, `--no-files`, no host socket, `--no-agent`, `--no-exec`); `ping` is always on. SOCKS is not a feature: it is a separate path (`/socks`) with its own switch. `Hello.features` (`host-events`, `agent`) says what the client can serve and is unchanged. The version check is at `supervisor.rs:2003-2011`; `tilr` refuses a `HelloAck` of another version (`crates/tilr/src/conn.rs:107-114`).

## Decisions

### D1. Version is the contract, features are switches
A v3 peer implements every message and field of v3. A later change that a peer cannot ignore (a new request a client relies on, a changed meaning, a new close code) bumps the version. A change a peer may ignore (a new push, a new field with a harmless default) needs neither. A feature is listed only for a capability an operator or the environment can turn off, so a client checks it before sending that capability's requests.

### D2. No compatibility path
The server answers any other version with `Error` and closes, as it does today. No v2 fallback, no flag for v2-era clients: no client has been released.

### D3. Client changes land together
`tilr` and `tilr-driver` are in this workspace and move in the same change (they use the constant). The `server` plugin (`npm run proto`) and tilr-mac copy the schema and bump their version constant; the README lists them as needing v3.

### D4. The `*_bytes` twins stay
Paths keep a `string` and a `bytes` twin; when the twin is not empty the server goes by it (as today). Rejected: bytes-only paths. Reasons: it would change every `Fs*` message and every client's path handling for no gain in correctness (v3 already makes the twins mandatory, which is what the review's wrong-file case needs); the string stays useful for display and logs. A later bytes-only cleanup would be another version bump under D1.

### D5. Empty `client_id` and empty `code` stay legal
An empty `Hello.client_id` keeps meaning "no identity" (supersedes nobody, is superseded by nobody): scripts and one-shot tools (`tilr ls`, the test client) have no instance to name, and refusing them would be a behavior change with no failure to fix. An empty `KeyInput.code` keeps meaning "no physical key" (synthetic input, and `tilr`'s character keys, which it reads from its own terminal as characters): it is encoded as on a US layout. Only the wording changes: neither is "an older client" any more.

### D6. Planned feature flags are checked against D1
Open changes that plan a feature flag (`osc8-hyperlinks`, `reverse-clipboard`, `port-forwarding`, `read-only-viewer`, `quic-transport`, `sandboxed-pane`) apply D1 when they are made ready: a flag only for what the server can be run without.

## Reload and reconnect

No state crosses a reload. After a reload every client reconnects and says `Hello` again, so a v2 client is refused at its reconnect, not mid-connection. A reload from a v2 server into a v3 one is allowed (no state format change): its v2 clients are refused at the reconnect.
