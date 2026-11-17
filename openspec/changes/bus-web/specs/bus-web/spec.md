## ADDED Requirements

### Requirement: A browser page is a bus participant
A page connecting over WebSocket with subprotocol `tilr.bus.v1` and a credential in `Hello` SHALL be a participant like any other: it SHALL be able to call methods, open streams, subscribe to events and provide methods, within its token's rights.

**Verified by**: `bus_web::a_page_calls_subscribes_and_provides` (a WebSocket client sending `Origin: tilr://app`).

#### Scenario: Plugin talks to Python
- **WHEN** a page with `call = ["nlp.*"]` calls `nlp.upper` provided by a Python instance with payload `hi`
- **THEN** it receives `HI`

#### Scenario: Python calls the plugin
- **WHEN** a page with `provide = ["ui.*"]` provides `ui.notify` and a Python instance with `call = ["ui.notify"]` calls it with `done`
- **THEN** the page's handler receives `done` and its reply reaches the Python instance

### Requirement: Browser origins are checked on the bus handshake
A WebSocket handshake that carries an `Origin` header SHALL be accepted only when the origin is in the allowed list (`--allow-origin`, else `allow_origins` in `server.toml`, else `tilr://app`, `tilr://tilr`, `http://127.0.0.1:7891`, `http://localhost:7891`); otherwise the server MUST answer HTTP 403 and log `rejected connection from disallowed origin`. A handshake without `Origin` MUST pass this check.

**Verified by**: `bus_web::only_allowed_origins_connect`.

#### Scenario: Page from another site
- **WHEN** a server started with `--allow-origin http://app.test` gets a handshake with `Origin: http://evil.test`
- **THEN** the handshake fails with 403

### Requirement: A token can be bound to an origin
A token minted with `--origin ORIGIN` (or attenuated with `check if origin("ORIGIN")`) SHALL authorize only on a connection whose handshake carried that `Origin`; on any other connection, including one without `Origin`, it MUST be `denied` with reason `origin`.

**Verified by**: `bus_web::a_token_bound_to_an_origin_works_only_there`.

#### Scenario: Copied into a script
- **WHEN** a token minted with `--origin tilr://app` is used by a program that sends no `Origin`
- **THEN** its `Hello` is answered `ConnectionError{code: "denied"}` whose message names `origin`

### Requirement: A provider that is gone makes calls unavailable
When no connection provides a method, a call to it SHALL be answered `unavailable` naming the method; when a providing page's connection ends, its calls in flight SHALL be answered `unavailable`.

**Verified by**: `bus_web::a_closed_window_makes_its_methods_unavailable`.

#### Scenario: Window closed during a call
- **WHEN** a page provides `ui.pick_file`, an instance calls it, and the page closes before answering
- **THEN** the instance receives `unavailable`

### Requirement: Two windows with one provider
Of several connections providing a method with `select` `LAST_ACTIVE`, a call without a target SHALL go to the one that sent `Activate` last, else to the one that connected last; when that connection ends, the next call SHALL go to the remaining one; events SHALL reach every subscribed window.

**Verified by**: `bus_web::two_windows_one_provider`.

#### Scenario: Focus moves
- **WHEN** windows `a` and `b` provide `ui.notify` with `LAST_ACTIVE`, `b` sends `Activate`, a call is made, then `b` closes and another call is made
- **THEN** the first call reaches `b` and the second reaches `a`

### Requirement: A page keeps its identity and renews its token
A page's identity SHALL be the participant named in its token (`token:<name>`), the same on every connection and window that uses a token of that name; panes it creates SHALL be its own (`created_by` `token:<name>`) across reconnects. A page SHALL be able to renew its token with `bus.renew` and present the new one with `CredentialUpdate` without reconnecting.

**Verified by**: `bus_web::a_page_keeps_its_identity_and_renews_its_token`.

#### Scenario: Reconnect with a renewed token
- **WHEN** a page with `pane = ["profile:demo", "access:own"]` creates a pane, renews its token with `bus.renew`, closes its connection and connects again with the renewed token
- **THEN** `pane.list` from the page lists that pane with `created_by` `token:<name>`

#### Scenario: Renewal on the open connection
- **WHEN** a page sends the token returned by `bus.renew` in `CredentialUpdate`
- **THEN** its connection stays open and its next call is allowed
