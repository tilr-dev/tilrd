# Design

## Context

At 47a8557 `PROTOCOL_VERSION` is 2 and `HelloAck.features` is `clipboard`, `ping`, `files`, `host-events`, `agent`, `exec` (`supervisor.rs:1906-1911`). `clipboard`, `files`, `host-events`, `agent` and `exec` can be off (`--no-clipboard`, `--no-files`, no host socket, `--no-agent`, `--no-exec`); `ping` is always on. SOCKS is not a feature: it is a separate path (`/socks`) with its own switch. `Hello.features` (`host-events`, `agent`) says what the client can serve and is unchanged.

## Decisions

### D1. Version is the contract, features are switches
A v3 peer implements every message and field of v3. A later change that a peer cannot ignore (a new request a client relies on, a changed meaning, a new close code) bumps the version. A change a peer may ignore (a new push, a new field with a harmless default) needs neither. A feature is listed only for a capability an operator or the environment can turn off, so a client checks it before sending that capability's requests.

### D2. No compatibility path
The server answers any other version with `Error` and closes, as it does today. No v2 fallback, no flag for v2-era clients: no client has been released.

### D3. Client changes land together
`tilr` and `tilr-driver` are in this workspace and move in the same change. The `server` plugin (`npm run proto`) and tilr-mac copy the schema and bump their version constant; they are listed in the README as needing v3.

## Open questions (not decided here)

- `*_bytes` twins: with no old clients, paths could be bytes only (one field, no string twin) and the string kept only in answers for display. That would change every `Fs*` message; decide before v3 is released, or leave the twins and bump again later.
- Should the server refuse a `Hello` with an empty `client_id`, and treat a `KeyInput` without `code` as an error? Today an empty `client_id` neither supersedes nor is superseded, and an empty `code` is encoded as on a US layout; this change keeps both.
- Open changes that plan a feature flag (`osc8-hyperlinks`, `reverse-clipboard`, `port-forwarding`, `read-only-viewer`, `quic-transport`, `sandboxed-pane`) are checked against D1 when they are designed.

## Reload and reconnect

No state crosses a reload. After a reload every client reconnects and says `Hello` again, so a v2 client is refused at its reconnect, not mid-connection.
