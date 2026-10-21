# Design

## Context

At 1163abd the server reads one token (`crates/tilrd/src/auth.rs`) and checks it in `Hello` on TCP (`crates/tilrd/src/supervisor.rs:2010-2030`); the Unix socket asks none. Every message a greeted client sends is handled. `HelloAck.features` lists `clipboard`, `ping`, `files`, `host-events`, `agent`, `exec` (the protocol version policy in `openspec/config.yaml` excludes `ping` because it is mandatory).

## Decisions

### D1. A second token, read at each Hello
`<state>/viewer-token` (64 hex characters, 0600, like `token`) is made only on request (`tilrd token --viewer`), so a server has no viewers until the user creates the token. The supervisor reads the file at each TCP `Hello` (one small read per connection), so creating, replacing or deleting it takes effect without a restart. The comparison is constant-time as for the token. A `Hello` whose token is the full token wins if both files were ever equal (not possible: both random). Rejected: a role flag in `Hello` (the note: a property of the connection, not something the client sets); scopes inside one token (needs a token format).

### D2. One gate in the supervisor
The connection carries `read_only`. Before dispatching a client message the supervisor checks it against an allow list (`ListPanes`, `Subscribe`, `Unsubscribe`, `Ping`); anything else is answered `Error { message: "this connection is read-only" }` and dropped. An allow list (not a deny list) means a message type added later is refused for viewers until someone allows it. Fan-out of `HostEvent`, agent streams and `ClipboardWrite` skips read-only connections. `HelloAck.features` is empty for them (none of those capabilities is offered on this connection).

### D3. Size and view
A viewer cannot `Resize` or `Scroll`, so it cannot shrink or move another user's pane (the note's "does not affect the pane's size"). It counts in `PaneInfo.viewers` and appears in `viewed_by` with `read_only` true.

### D4. Replacing the token
`tilrd token --viewer --new` writes a new token (temp file, rename) and pings the running server over its Unix socket (server command `token --viewer --new` runs in the server, as `tilr token` does), which closes read-only connections greeted with the old token (`Error` "the viewer token was replaced", close 1000). No new close code.

## Reload and reconnect

The viewer token is a file; nothing new crosses a reload. After the exec every client reconnects and is greeted again; a viewer presenting the same token is read-only again. A reconnecting viewer with a `client_id` supersedes only its own earlier connection (same token, as today).

## Backpressure

A viewer gets frames like any subscriber (interactive lane, lagging clients skip frames). Refusals are one small `Error` per refused message on the interactive lane; a client flooding refused messages is dropped as too slow (4001) like any client whose queue fills.
