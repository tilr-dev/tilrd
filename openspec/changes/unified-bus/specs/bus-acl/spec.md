## ADDED Requirements

### Requirement: The root key is made once and kept for the user only
The server SHALL, on its first start, make an Ed25519 key and keep it in `<state dir>/bus-root.key` with mode 0600, reuse it on every later start and reload, and MUST NOT replace an existing key file except on `bus.rotate_key`.

**Verified by**: `bus_acl::the_root_key_is_made_once`.

#### Scenario: First and second start
- **WHEN** the server starts with an empty state directory, stops and starts again
- **THEN** `bus-root.key` exists with mode 0600 and has the same content after the second start

### Requirement: The owner holds every right and nobody else does without a token
A connection on the Unix socket of the server's user, or on TCP with the access token, that presents no `credential` SHALL be the principal `owner`, allowed every call, provide, emit, subscribe and control. Every other participant SHALL be authorized only by a token of this server: an instance by the one minted at its start, an outside participant by `Hello.credential`; a WebSocket connection on TCP with neither the access token nor a credential MUST be refused.

**Verified by**: `bus_acl::the_owner_holds_every_right`, `bus_acl::a_tcp_participant_needs_a_token`.

#### Scenario: Owner
- **WHEN** a client on the Unix socket says `Hello` with no credential and calls `bus.instances`
- **THEN** it receives the list and its `HelloAck.identity` is `owner`

#### Scenario: Nothing on TCP
- **WHEN** a client on the TCP port says `Hello` with neither `access_token` nor `credential`
- **THEN** it receives `ConnectionError` with code `denied` and the connection closes

### Requirement: An instance gets a token for its configured rights
At each start of an instance the server SHALL mint a token naming the participant `instance:<name>`, holding the instance's `[instances.<name>.rights]` (keys `call`, `provide`, `emit`, `subscribe`, `control`, `pane`), a check binding it to this server and an expiry 24 h ahead, and send it in `HelloAck.credential`. An instance without `rights` SHALL be allowed nothing.

**Verified by**: `bus_acl::an_instance_may_do_what_its_rights_list`.

#### Scenario: Namespace right
- **WHEN** `t-1` has `call = ["test.*"]` and calls `test.echo` and `other.echo` (provided by `t-2`)
- **THEN** `test.echo` is answered and `other.echo` is `denied`

#### Scenario: No rights
- **WHEN** `t-1` has no `rights` and calls `test.echo`
- **THEN** it is `denied`

### Requirement: Providing, emitting and subscribing need rights
A `Hello` that provides a method not covered by its token's `provide` rights MUST be refused with `ConnectionError{code: "denied"}` naming the method; an `Event` whose subject is not covered by `emit` MUST be dropped and audited; a `Subscribe` whose pattern is not inside a `subscribe` right MUST be answered `denied`. The namespaces `bus`, `pane`, `server`, `fs`, `exec` and `clipboard` MUST NOT be emitted by anyone but `tilrd`.

**Verified by**: `bus_acl::providing_emitting_and_subscribing_need_rights`.

#### Scenario: Subscribe outside the right
- **WHEN** `t-1` has `subscribe = ["test.*"]` and subscribes to `test.*` and to `pane.*`
- **THEN** the first is answered with an empty reply and the second is `denied`

#### Scenario: Forged core event
- **WHEN** `t-1` with `emit = ["*"]` emits `pane.exited`
- **THEN** no subscriber receives it and the log has a `bus audit: deny` line for it

### Requirement: Control methods need control rights
`bus.instances` and `bus.methods` SHALL need `control` `list`; `bus.start`, `bus.stop` and `bus.reload` that action on every instance they act on; `bus.reload_config`, `bus.token`, `bus.revoke`, `bus.rotate_key` and `server.reload` their own right; otherwise the call MUST be `denied` and change nothing.

**Verified by**: `bus_acl::control_methods_need_control_rights`.

#### Scenario: Reload only itself
- **WHEN** `t-1` has `control = ["reload:t-1"]` and calls `bus.reload` for `t-1`, then for `t-2`
- **THEN** the first is answered, the second is `denied` and `t-2` keeps its pid

### Requirement: A token works only on the server that made it
A token minted by another server, one whose signature does not verify against this server's root key, or one that is malformed, MUST make the call, the `Hello` or the subscription `denied`.

**Verified by**: `bus_acl::a_token_of_another_server_is_refused`.

#### Scenario: Another machine's token
- **WHEN** the owner mints a token on server A with `tilr bus token --name x --call 'test.*'` and a call on server B carries it as `credential`
- **THEN** server B answers `denied`

### Requirement: A per-call credential is a bearer
A `Call` with a non-empty `credential` SHALL be authorized by that token alone, whatever the caller's own token allows or forbids; `from` SHALL still be the caller's identity.

**Verified by**: `bus_acl::a_per_call_credential_is_a_bearer`.

#### Scenario: Narrow caller, wide credential
- **WHEN** `t-1` has no rights and calls `test.echo` with a credential the owner minted for `call = ["test.echo"]`
- **THEN** the call is answered, and the callee sees `from` `instance:t-1`

#### Scenario: Wide caller, narrow credential
- **WHEN** the owner calls `test.whoami` with a credential minted for `test.echo` only
- **THEN** the call is `denied`

### Requirement: A holder can narrow a token without the server
A token holder SHALL be able to append a block of checks (on `method`, `target`, `subject`, `origin`, `time`) without the root key, and the server SHALL enforce every block; an appended block MUST NOT add rights. More than 8 blocks or more than 16 KiB SHALL be `denied`.

**Verified by**: `bus_acl::an_attenuated_token_only_narrows`.

#### Scenario: Narrowed to one target
- **WHEN** `t-1` attenuates its token with `check if target("instance:t-2")` and presents it when calling `test.echo` on `t-2` and on `t-3`
- **THEN** the call to `t-2` is answered and the one to `t-3` is `denied`

#### Scenario: A block that adds a right
- **WHEN** an appended block contains the fact `call_any()`
- **THEN** a call outside the authority block's rights is still `denied`

### Requirement: Tokens expire after a day and are renewed
Every token SHALL expire at most 24 h after it was minted, and a call or subscription carrying an expired token MUST be `denied` with reason `expired`, at most 1 s after the expiry. An instance's token SHALL be renewed by the server with `CredentialUpdate` 12 h after it was minted while the instance runs. A connection whose token expires without a `CredentialUpdate` from the participant SHALL be closed with code 4003 `credential`.

**Verified by**: `bus_acl::an_expired_token_is_refused`, `bus_acl::an_instance_token_is_renewed`, `bus_acl::an_outside_token_is_renewed_on_its_connection` (controlled time).

#### Scenario: Long-running instance
- **WHEN** the test clock moves 13 h ahead while `t-1` runs
- **THEN** `t-1` has received `CredentialUpdate` and its calls are still allowed after 25 h

### Requirement: bus.renew gives a fresh token for an unattenuated one
`bus.renew` SHALL answer, for a valid, unrevoked token without appended blocks, a new token with the same rights and a new expiry, and revoke the old one; for an attenuated token it SHALL answer `bad_request`.

**Verified by**: `bus_acl::an_outside_token_is_renewed_on_its_connection` (controlled time).

#### Scenario: Attenuated token
- **WHEN** a participant calls `bus.renew` with a token it attenuated
- **THEN** the reply is `bad_request` and says to renew the token it attenuated from

#### Scenario: Outside participant renews
- **WHEN** a WebSocket participant calls `bus.renew` 20 h after its token was minted and sends the result in `CredentialUpdate`
- **THEN** its connection is still open 30 h after the first token was minted, and the first token is `denied` with `revoked`

#### Scenario: Not renewed
- **WHEN** a WebSocket participant's token expires and it sent no `CredentialUpdate`
- **THEN** its connection is closed with code 4003

### Requirement: The owner mints tokens
`tilr bus token --name NAME [--call P…] [--provide P…] [--emit P…] [--subscribe P…] [--control A[:INSTANCE]…] [--pane R…] [--origin ORIGIN] [--ttl DURATION]` SHALL call `bus.token` and print one token (base64url) on stdout; the token's participant SHALL be `token:NAME`; it SHALL fail for a ttl over 24 h.

**Verified by**: `client::tilr_bus_token_mints_a_token_that_works`.

#### Scenario: Mint and use
- **WHEN** the user runs `tilr bus token --name s --call test.echo --ttl 1h` and a call carries the printed token
- **THEN** `test.echo` is answered and `test.whoami` is `denied`

#### Scenario: Too long
- **WHEN** the user runs `tilr bus token --name s --call test.echo --ttl 48h`
- **THEN** it exits 1 and says the ttl is at most 24h

### Requirement: Revoked tokens stop working at once
After `bus.revoke` (`tilr bus revoke TOKEN | --id REVOCATION_ID`), every call carrying that token or one attenuated from it MUST be `denied` with reason `revoked`, also on the next call and after a restart of the server, and a connection that said `Hello` with it SHALL be closed with code 4003; an instance's token SHALL be revoked when its process ends.

**Verified by**: `bus_acl::a_revoked_token_stops_working_at_once`, `bus_acl::a_stopped_instances_token_is_revoked`.

#### Scenario: Revoke while cached
- **WHEN** a minted token is used for 10 calls in a second, then revoked, then used again
- **THEN** the call after the revocation is `denied` with `revoked`, and so is a call after the server restarts

#### Scenario: Instance stopped
- **WHEN** `t-1` handed its token to the test as text, then `tilr bus stop t-1` runs
- **THEN** a call carrying that token is `denied` with `revoked`

### Requirement: Rotating the root key ends every token at once
`bus.rotate_key` (`tilr bus rotate-key`) SHALL replace `bus-root.key`; every token made with the old key MUST then be `denied`, with no period in which both keys are accepted; every running instance SHALL receive a token of the new key by `CredentialUpdate` and keep its pid; every connection that said `Hello` with a minted token SHALL be closed with code 4003.

**Verified by**: `bus_acl::rotating_the_root_key_ends_every_token`.

#### Scenario: Old minted token
- **WHEN** a minted token works and a WebSocket participant is connected with it, then `tilr bus rotate-key` runs
- **THEN** that token is `denied`, the participant's connection is closed with 4003, and `t-1` has the same pid and its calls are allowed

### Requirement: Authorization fails closed and is bounded
Any failure to decide — a Datalog run over 1 ms, more than 1000 facts or 100 iterations, a parse or signature error — MUST make the call `denied` (reason `authorization limit reached` or the error), never allowed. A repeated decision for the same token and request SHALL be answered from a cache for at most 1 s, and the cache SHALL be cleared on revocation, key rotation and `bus.reload_config`.

**Verified by**: `bus_acl::authorization_that_runs_too_long_is_denied`, `bus_acl::a_revocation_clears_cached_decisions`.

#### Scenario: Run limit
- **WHEN** a holder appends a block whose rules generate more than 1000 facts and presents the token
- **THEN** the call is `denied` with `authorization limit reached`

### Requirement: Denied and control calls are audited
The server log SHALL have a `bus audit:` line for every denied call, provide, emit and subscription (caller, token revocation id prefix, kind, method or subject, target, reason) and for every control, mint, renew, revoke and key-rotation call, allowed or not; token bytes MUST NOT appear in the log.

**Verified by**: `bus_acl::denied_and_control_calls_are_audited`.

#### Scenario: A denial
- **WHEN** `t-1` is denied `test.whoami`
- **THEN** the log has a line with `bus audit: deny`, `from=instance:t-1`, `method=test.whoami` and a reason, and no line contains the token
