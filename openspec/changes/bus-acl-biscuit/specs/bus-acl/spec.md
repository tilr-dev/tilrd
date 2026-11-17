## ADDED Requirements

### Requirement: The root key is made once and kept for the user only
The server SHALL, on its first start, make an Ed25519 key and keep it in `<state dir>/bus-root.key` with mode 0600, reuse it on every later start and reload, and MUST NOT replace an existing key file except on `bus.rotate_key`.

**Verified by**: `bus_acl::the_root_key_is_made_once`.

#### Scenario: First and second start
- **WHEN** the server starts with an empty state directory, stops and starts again
- **THEN** `bus-root.key` exists with mode 0600 and has the same content after the second start

### Requirement: An instance gets a token for its configured rights
At each start of an instance the server SHALL mint a token with the instance's name, its `[instances.<name>.rights]` (`call`: exact methods, `ns.*` or `*`; `control`: `action` or `action:instance-glob`), a check binding it to this server and an expiry 24 h ahead, and send it in `HelloAck.credential`. An instance without `rights` SHALL be allowed no call.

**Verified by**: `bus_acl::an_instance_may_call_what_its_rights_list`.

#### Scenario: Namespace right
- **WHEN** `t-1` has `call = ["test.*"]` and calls `test.echo` and `other.echo` (provided by `t-2`)
- **THEN** `test.echo` is answered and `other.echo` is `denied`

#### Scenario: No rights
- **WHEN** `t-1` has no `rights` and calls `test.echo`
- **THEN** it is `denied`

#### Scenario: The old key
- **WHEN** `bus.toml` still uses `calls = ["test.*"]`
- **THEN** the config is invalid and the reason says `calls` was replaced by `rights`

### Requirement: Control methods need control rights
`bus.instances` and `bus.methods` SHALL need `control` `list`; `bus.start`, `bus.stop` and `bus.reload` SHALL need that action on every instance they act on; `bus.reload_config`, `bus.token`, `bus.revoke` and `bus.rotate_key` SHALL need their own right; otherwise the call MUST be `denied` and change nothing.

**Verified by**: `bus_acl::control_methods_need_control_rights`.

#### Scenario: Reload only itself
- **WHEN** `t-1` has `control = ["reload:t-1"]` and calls `bus.reload` for `t-1`, then for `t-2`
- **THEN** the first is answered, the second is `denied` and `t-2` keeps its pid

### Requirement: A token works only on the server that made it
A token minted by another server, one whose signature does not verify against this server's root key, or one that is malformed, MUST make the call `denied`.

**Verified by**: `bus_acl::a_token_of_another_server_is_refused`.

#### Scenario: Another machine's token
- **WHEN** the owner mints a token on server A with `tilr bus token --call 'test.*'` and a call on server B carries it as `credential`
- **THEN** server B answers `denied`

### Requirement: A per-call credential authorizes that call
A `Call` (or `BusCall`) with a non-empty `credential` SHALL be authorized by that token alone, whatever the caller's own token allows; `from` SHALL still be the caller's identity.

**Verified by**: `bus_acl::a_per_call_credential_authorizes_that_call`.

#### Scenario: Narrow caller, wide credential
- **WHEN** `t-1` has no rights and calls `test.echo` with a credential the owner minted for `call = ["test.echo"]`
- **THEN** the call is answered, and the callee sees `from` `t-1`

#### Scenario: Wide caller, narrow credential
- **WHEN** the owner calls `test.whoami` with a credential minted for `test.echo` only
- **THEN** the call is `denied`

### Requirement: A holder can narrow a token without the server
A token holder SHALL be able to append a block of checks (on `method`, `target`, `time`) without the root key, and the server SHALL enforce every block; an appended block MUST NOT add rights. More than 8 blocks or more than 16 KiB SHALL be `denied`.

**Verified by**: `bus_acl::an_attenuated_token_only_narrows`.

#### Scenario: Narrowed to one target
- **WHEN** `t-1` attenuates its token with `check if target("t-2")` and presents it when calling `test.echo` on `t-2` and on `t-3`
- **THEN** the call to `t-2` is answered and the one to `t-3` is `denied`

#### Scenario: A block that adds a right
- **WHEN** an appended block contains the fact `call_any()`
- **THEN** a call outside the authority block's rights is still `denied`

### Requirement: Tokens expire
A call carrying a token whose expiry has passed MUST be `denied` with reason `expired`, at most 1 s after the expiry. `bus.token` SHALL require a `ttl` of at most 30 days (default 1 h). An instance's token SHALL be renewed by the server with `CredentialUpdate` before it expires, for as long as the instance runs.

**Verified by**: `bus_acl::an_expired_token_is_refused`, `bus_acl::an_instance_token_is_renewed` (controlled time).

#### Scenario: Minted token runs out
- **WHEN** the owner mints a token with `--ttl 2s` and it is used 4 s later
- **THEN** the call is `denied` with `expired`

#### Scenario: Long-running instance
- **WHEN** the test clock moves 13 h ahead while `t-1` runs
- **THEN** `t-1` has received `CredentialUpdate` and its calls are still allowed after 25 h

### Requirement: The owner mints tokens
`tilr bus token --call PATTERN… [--control ACTION[:INSTANCE]…] [--ttl DURATION]` SHALL call `bus.token` and print one token (base64url) on stdout; it SHALL fail for a ttl over 30 days.

**Verified by**: `client::tilr_bus_token_mints_a_token_that_works`.

#### Scenario: Mint and use
- **WHEN** the user runs `tilr bus token --call test.echo --ttl 1h` and a call carries the printed token
- **THEN** `test.echo` is answered and `test.whoami` is `denied`

### Requirement: Revoked tokens stop working at once
After `bus.revoke` (`tilr bus revoke TOKEN | --id REVOCATION_ID`), every call carrying that token or one attenuated from it MUST be `denied` with reason `revoked`, also on the next call and after a restart of the server; an instance's token SHALL be revoked when its process ends.

**Verified by**: `bus_acl::a_revoked_token_stops_working_at_once`, `bus_acl::a_stopped_instances_token_is_revoked`.

#### Scenario: Revoke while cached
- **WHEN** a minted token is used for 10 calls in a second, then revoked, then used again
- **THEN** the call after the revocation is `denied` with `revoked`, and so is a call after the server restarts

#### Scenario: Instance stopped
- **WHEN** `t-1` handed its token to the test as text, then `tilr bus stop t-1` runs
- **THEN** a call carrying that token is `denied` with `revoked`

### Requirement: Rotating the root key ends every token
`bus.rotate_key` (`tilr bus rotate-key`) SHALL replace `bus-root.key`; every token made with the old key MUST then be `denied`, and every running instance SHALL be reloaded one at a time with a token of the new key.

**Verified by**: `bus_acl::rotating_the_root_key_ends_every_token`.

#### Scenario: Old minted token
- **WHEN** a minted token works, then `tilr bus rotate-key` runs
- **THEN** that token is `denied`, `t-1` has a new pid and its calls are allowed

### Requirement: Authorization fails closed
Any failure to decide — a Datalog run over 1 ms, more than 1000 facts or 100 iterations, a parse or signature error — MUST make the call `denied` (reason `authorization limit reached` or the error), never allowed.

**Verified by**: `bus_acl::authorization_that_runs_too_long_is_denied`.

#### Scenario: Run limit
- **WHEN** a holder appends a block whose rules generate more than 1000 facts and presents the token
- **THEN** the call is `denied` with `authorization limit reached`

### Requirement: Denied and control calls are audited
The server log SHALL have a `bus audit:` line for every denied call (caller, token revocation id prefix, method, target, reason) and for every control, mint, revoke and key-rotation call, allowed or not; token bytes MUST NOT appear in the log.

**Verified by**: `bus_acl::denied_and_control_calls_are_audited`.

#### Scenario: A denial
- **WHEN** `t-1` is denied `test.whoami`
- **THEN** the log has a line with `bus audit: deny`, `from=t-1`, `method=test.whoami` and a reason, and no line contains the token
