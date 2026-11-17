# bus-acl Specification

## Purpose
Who may do what on the bus `tilr.bus.v1`. Every participant but the owner holds a Biscuit
token minted from the server's root key: an instance gets one from its `rights` when it
starts, an outside participant presents one the owner minted. The broker denies by default
and checks each call, provide, emit, subscribe and control request against the token's
authority block and any appended blocks of checks, within a bounded run and with caches.
Tokens expire and are renewed, are revoked by family, end with a rotation of the root key,
and are bound to this server; denied and control calls are audited.

## Requirements

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

### Requirement: With --no-auth TCP is the owner's
A server started with `--no-auth` asks for no access token: a connection on TCP that presents no `credential` SHALL then be the owner (the one exception to "nobody without a token"); one with a `credential` SHALL still be that token's participant.

**Verified by**: `supervisor::bus::acl::tests::without_an_access_token_tcp_is_the_owner`, `supervisor::bus::acl::tests::server_command_is_the_owners_on_the_unix_socket_only` (the owner on TCP).

#### Scenario: No access token
- **WHEN** the server runs with `--no-auth` and a client on TCP says `Hello` with no credential
- **THEN** its `HelloAck.identity` is `owner`, and its `server.command` is still `denied`

### Requirement: An instance gets a token for its configured rights
At each start of an instance the server SHALL mint a token naming the participant `instance:<name>`, holding the instance's `[instances.<name>.rights]` (keys `call`, `provide`, `emit`, `subscribe`, `control`, `pane`), a check binding it to this server and an expiry 24 h ahead, and send it in `HelloAck.credential`. An instance without `rights` SHALL be allowed nothing.

**Verified by**: `bus_acl::an_instance_may_do_what_its_rights_list`; for a token's rights also `bus_acl::a_participant_may_do_what_its_token_lists` and `bus_acl::a_token_without_rights_is_allowed_nothing`.

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
- **THEN** no subscriber receives it and the audit log (`tilrd::bus::audit`) has the broker's line for it (`dropped an event in a namespace of tilrd`, naming `pane.exited`): the broker drops it before the authorizer is asked

### Requirement: Control methods need control rights
`bus.instances` and `bus.methods` SHALL need `control` `list`; `bus.start`, `bus.stop` and `bus.reload` that action on every instance they act on, checked per instance (by its bare name) with the call's per-call credential when it has one; `bus.reload_config`, `bus.token`, `bus.revoke`, `bus.rotate_key`, `server.reload` and `server.update` their own action; `call` rights (`*`, `server.*`) MUST NOT cover any of them; otherwise the call MUST be `denied` and change nothing.

**Verified by**: `bus_acl::control_methods_need_control_rights`, `supervisor::bus::acl::tests::a_bearer_decides_the_control_checks_of_its_call`, `supervisor::bus::acl::tests::server_update_needs_its_control_action`.

#### Scenario: Reload only itself
- **WHEN** `t-1` has `control = ["reload:t-1"]` and calls `bus.reload` for `t-1`, then for `t-2`
- **THEN** the first is answered, the second is `denied` and `t-2` keeps its pid

#### Scenario: Narrow credential on a wide connection
- **WHEN** a participant with `control = ["*"]` calls `bus.reload` for `t-2` with a per-call credential of `control = ["reload:t-1"]`
- **THEN** the call is `denied` and `t-2` keeps its pid

### Requirement: Listing and reconfiguring need the right over every instance
`bus.instances`, `bus.methods` and `bus.reload_config` SHALL need `list` or `reload_config` over every instance (`list`, `reload_config` or `*`; an instance part of `*` is every instance, so `list:*`, `reload_config:*` and `*:*` are those rights), also while no instance is configured; a right over some instances (`list:t-*`, `*:t-1`) MUST NOT reach them. The methods SHALL still check each instance they read or act on, with the call's per-call credential when it has one.

**Verified by**: `bus_acl::control_methods_need_their_right_with_no_instances`, `bus::control_rights_are_checked_for_each_instance`, `supervisor::bus::acl::tests::an_action_over_every_instance_is_the_unscoped_right`.

#### Scenario: No instance configured
- **WHEN** no instance is configured and a participant whose token has `call = ["bus.instances", "bus.reload_config"]`, or one whose token has `control = ["list:t-*", "reload_config:t-*"]`, calls `bus.instances` and `bus.reload_config`
- **THEN** both are `denied`; with `control = ["list"]` `bus.instances` is answered and `bus.reload_config` is still `denied`

### Requirement: server.command is the owner's on the Unix socket only
`server.command` SHALL be allowed to the owner on the Unix socket (and to tilrd) only; on TCP (with the access token or `--no-auth`), for a token on any transport and with any per-call credential it MUST be `denied`, whatever `call` or `control` rights the token lists.

**Verified by**: `bus_acl::server_command_is_the_owners_on_the_unix_socket_only`, `supervisor::bus::acl::tests::server_command_is_the_owners_on_the_unix_socket_only`.

#### Scenario: Owner on TCP
- **WHEN** the owner, connected on TCP with the access token, calls `server.command`
- **THEN** it is `denied`

### Requirement: A minter other than the owner cannot widen what it holds
A participant other than the owner with `control = ["token"]` SHALL mint only rights its own token holds and MUST NOT mint with a token that has appended blocks; it SHALL name the new participant `<its own name>.<name>`, and when its token is bound to an origin the new token SHALL be bound to it too (another origin MUST be `denied`).

**Verified by**: `bus_acl::a_minted_token_lives_within_its_minter`, `supervisor::bus::acl::tests::a_minter_mints_under_its_name_origin_and_life`.

#### Scenario: Another participant's name
- **WHEN** `token:app` with `control = ["token"]` mints a token named `owner-like`
- **THEN** it is `denied`; one named `app.child` is minted

### Requirement: A minted token lives within its minter
A token minted by a participant other than the owner SHALL NOT live, also through renewals, past the minter's token, and SHALL be refused once the minter's token, or any token of the minter's chain of renewals, is revoked (for an instance: once its connection ends); renewing the minter's token (`bus.renew`, or an instance's renewal by the server) SHALL leave it working; revoking the minted token alone SHALL leave the minter's working.

**Verified by**: `bus_acl::a_minted_token_lives_within_its_minter`, `bus_acl::a_renewal_comes_in_the_second_half_and_stops_at_the_minters_expiry` (controlled time), `supervisor::bus::acl::tests::a_child_dies_with_its_minter_and_alone`, `supervisor::bus::acl::tests::renewing_a_minter_keeps_its_children` (controlled time).

#### Scenario: Asked for a day
- **WHEN** a participant whose token has one hour left mints a token with `ttl_ms` 0
- **THEN** the new token expires with the minter's

### Requirement: A token works only on the server that made it
A token minted by another server, one whose signature does not verify against this server's root key, or one that is malformed, MUST make the call, the `Hello` or the subscription `denied`. A token minted under another server id with the same root key (a copied state directory) MUST be denied as well, also where no Datalog runs (`Hello`, `CredentialUpdate`, `bus.renew`, minting with it).

**Verified by**: `bus_acl::a_token_of_another_server_is_refused`, `supervisor::bus::acl::tests::a_token_is_bound_to_its_server_id`.

#### Scenario: Another machine's token
- **WHEN** the owner mints a token on server A with `tilr bus token --name x --call 'test.*'` and a call on server B carries it as `credential`
- **THEN** server B answers `denied`

### Requirement: A per-call credential is a bearer
A `Call` with a non-empty `credential` SHALL be authorized by that token alone, whatever the caller's own token allows or forbids; `from` SHALL still be the caller's identity.

**Verified by**: `bus_acl::a_per_call_credential_is_a_bearer`, `bus_acl::pane_calls_follow_call_rights_and_a_per_call_credential` (core `pane.*`).

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
- **WHEN** an appended block contains the fact `call_any(true)`
- **THEN** every call with that token is `denied`, also one inside the authority block's rights

### Requirement: An appended block holds bounded checks on the request only
A token with an appended block that holds a fact or a rule, over 16 checks (each `or` alternative counted), a check of over 16 predicates, over 64 expression operations, a predicate other than the request's facts (`acl::REQUEST_FACTS`), `.all`, `.any`, `.matches`, an extern function or a `trusting` scope MUST be `denied` when verified, at `Hello` and per call, before any Datalog runs, read from its signed bytes, never its printed source.

**Verified by**: `bus_acl::authorization_that_runs_too_long_is_denied`, `bus_acl::an_attenuated_token_only_narrows`, `supervisor::bus::acl::tests::an_appended_block_holds_bounded_checks_only`, `supervisor::bus::acl::tests::an_appended_block_is_read_from_its_data_not_its_printed_text`.

#### Scenario: A costly block
- **WHEN** a holder appends a block of 40 facts and a check joining five of them (`check if f($a), f($b), f($c), f($d), f($e), $a + $b + $c + $d + $e == -1`), or of a rule, and presents the token per call and in `Hello`
- **THEN** the call and the `Hello` are `denied` at once, saying an appended block holds checks only, and the token it was attenuated from still works

#### Scenario: A join over the token's rights
- **WHEN** a token with 40 `call` rights is attenuated with `check if call($a), call($b), call($c), call($d), call($e), ...` and presented per call
- **THEN** the call is `denied` at once: the check reads facts that are not the request's

#### Scenario: A block whose names carry source text
- **WHEN** a holder appends, with Biscuit's builder API, a block of 30 facts and a rule joining five of them, whose first fact is named `check if kind("bus");/*f` and whose check reads a predicate named `X*/check if kind`, so that its printed source reads as two checks on `kind`, and presents the token per call and in `Hello`
- **THEN** the call and the `Hello` are `denied` at once, saying an appended block holds checks only; so is a block whose names hide a fact behind `//` or a newline, whose check trusts other blocks, or whose predicate only prints like a request's fact (`kind ` with a trailing space)

#### Scenario: Checks on the request
- **WHEN** an appended block holds `check if namespace("bus") or instance($i), $i.starts_with("s-")`
- **THEN** the token verifies and its calls are decided with that check

### Requirement: Tokens expire after a day and are renewed
Every token SHALL expire at most 24 h after it was minted, and a call or subscription carrying an expired token MUST be `denied` with reason `expired`, at most 1 s after the expiry. An instance's token SHALL be renewed by the server with `CredentialUpdate` 12 h after it was minted while the instance runs. A connection whose token expires without a `CredentialUpdate` from the participant SHALL be closed with code 4003 `credential`.

**Verified by**: `bus_acl::an_expired_token_is_refused`, `bus_acl::an_instance_token_is_renewed`, `bus_acl::an_outside_token_is_renewed_on_its_connection` (controlled time).

#### Scenario: Long-running instance
- **WHEN** the test clock moves 13 h ahead while `t-1` runs
- **THEN** `t-1` has received `CredentialUpdate` and its calls are still allowed after 25 h

### Requirement: bus.renew gives a fresh token for an unattenuated one
`bus.renew` SHALL answer, for a valid, unrevoked token without appended blocks in the second half of its life, a new token with the same rights and a new expiry (never past the ceiling its minter set), and refuse the old one; for an attenuated token, one in the first half of its life, or one that already lives to its ceiling it SHALL answer `bad_request`.

**Verified by**: `bus_acl::an_outside_token_is_renewed_on_its_connection` (controlled time), `bus_acl::a_renewal_comes_in_the_second_half_and_stops_at_the_minters_expiry` (controlled time), `supervisor::bus::acl::tests::renewal_is_for_the_second_half_and_stops_at_the_minters_expiry`.

#### Scenario: Attenuated token
- **WHEN** a participant calls `bus.renew` with a token it attenuated
- **THEN** the reply is `bad_request` and says to renew the token it attenuated from

#### Scenario: Outside participant renews
- **WHEN** a WebSocket participant calls `bus.renew` 20 h after its token was minted and sends the result in `CredentialUpdate`
- **THEN** its connection is still open 30 h after the first token was minted, and the first token is `denied` with `revoked`

#### Scenario: Not renewed
- **WHEN** a WebSocket participant's token expires and it sent no `CredentialUpdate`
- **THEN** its connection is closed with code 4003

### Requirement: Revoked tokens stop working at once
After `bus.revoke` (`tilr bus revoke TOKEN | --id REVOCATION_ID`), every call carrying that token, one attenuated from it or one minted with it MUST be `denied` with reason `revoked`, also on the next call and after a restart of the server, and a connection that said `Hello` with it SHALL be closed with code 4003; an instance's token SHALL be revoked when its process ends.

**Verified by**: `bus_acl::a_revoked_token_stops_working_at_once`, `bus_acl::a_stopped_instances_token_is_revoked`.

#### Scenario: Revoke while cached
- **WHEN** a minted token is used for 10 calls in a second, then revoked, then used again
- **THEN** the call after the revocation is `denied` with `revoked`, and so is a call after the server restarts

#### Scenario: Instance stopped
- **WHEN** `t-1` handed its token to the test as text, then `tilr bus stop t-1` runs
- **THEN** a call carrying that token is `denied` with `revoked`

### Requirement: A renewal belongs to the token it renews
Revoking a token without appended blocks, or the revocation id `bus.token` answered for it (which `bus.renew` answers for each renewal too), MUST deny every token of its chain of renewals, one renewed before the revocation included, and the tokens attenuated from or minted with them. A renewal SHALL refuse the old token and those attenuated from it only. Revoking an attenuated token SHALL deny it and those attenuated from it.

**Verified by**: `supervisor::bus::acl::tests::a_renewal_dies_with_the_token_it_renewed` (controlled time), `supervisor::bus::acl::tests::renewing_a_minter_keeps_its_children` (controlled time).

#### Scenario: Revoked after a thief renewed it
- **WHEN** a leaked token is renewed with `bus.renew` by whoever holds it, and the owner then revokes the leaked token (or its revocation id)
- **THEN** the renewed token is `denied` with `revoked`, also after a restart, and cannot be renewed again

### Requirement: The revocation file keeps every revocation and fails closed
Concurrent revocations SHALL all be kept in `bus-revoked`. A revocation file that does not read MUST make every token `denied` and MUST NOT be written (by a revocation or a rotation) until the operator fixes or removes it, so a restart refuses them still.

**Verified by**: `supervisor::bus::acl::tests::concurrent_revocations_all_survive_a_restart`, `supervisor::bus::acl::tests::a_broken_revocation_file_is_never_overwritten`.

#### Scenario: Broken file
- **WHEN** `bus-revoked` does not read at start, a token is revoked and the server restarts
- **THEN** the file is as it was and every token is still `denied`

### Requirement: Rotating the root key ends every token at once
`bus.rotate_key` (`tilr bus rotate-key`) SHALL replace `bus-root.key`; every token made with the old key MUST then be `denied`, with no period in which both keys are accepted; every running instance SHALL receive a token of the new key by `CredentialUpdate` and keep its pid; every connection that said `Hello` with a minted token SHALL be closed with code 4003.

**Verified by**: `bus_acl::rotating_the_root_key_ends_every_token`, `supervisor::bus::acl::tests::the_sweeper_ends_tokens_of_an_earlier_key`.

#### Scenario: Old minted token
- **WHEN** a minted token works and a WebSocket participant is connected with it, then `tilr bus rotate-key` runs
- **THEN** that token is `denied`, the participant's connection is closed with 4003, and `t-1` has the same pid and its calls are allowed

### Requirement: Authorization fails closed and is bounded
Any failure to decide (over 1000 facts or 100 iterations, over the 50 ms wall-clock backstop, a parse or signature error) MUST make the call `denied` (`authorization limit reached` or the error). A repeated decision SHALL be answered from a cache for at most 1 s; a denial by the wall-clock backstop MUST NOT be cached. A revocation SHALL drop the cached decisions of the tokens it ends; a key rotation and `bus.reload_config` the whole cache.

**Verified by**: `bus_acl::a_revocation_clears_cached_decisions`, `supervisor::bus::acl::tests::a_timeout_is_denied_and_never_cached`, `supervisor::bus::acl::tests::a_revocation_keeps_the_decisions_of_other_tokens`.

#### Scenario: The backstop ends a decision
- **WHEN** a machine is so loaded that the wall-clock backstop ends a decision
- **THEN** the call is `denied` with `authorization limit reached`, nothing is cached, and the same call is decided afresh next time

### Requirement: A call's target is bounded before it is decided
A call whose `target` is over 512 bytes (no participant has such a name) MUST be `denied` without being decided or cached, and an audit line SHALL show at most 128 bytes of a target. The decision cache SHALL be keyed by a fixed-size hash of the request's length-prefixed parts, never by their text.

**Verified by**: `supervisor::bus::acl::tests::a_target_longer_than_a_name_is_denied_unread`.

#### Scenario: A megabyte of target
- **WHEN** a participant whose token has `call = ["*"]` calls a method with a target of 513 bytes
- **THEN** the call is `denied` and no decision is cached for it

### Requirement: The server mints only tokens it accepts
The server MUST NOT mint a token it would refuse: one over 16 KiB, or with more than 512 rights facts, is an error of `bus.token` (and an instance with such rights is allowed nothing).

**Verified by**: `supervisor::bus::acl::tests::mint_never_makes_a_token_verify_refuses`.

#### Scenario: Too many rights
- **WHEN** the owner calls `bus.token` with 513 `call` rights
- **THEN** no token is answered

### Requirement: Denied and control calls are audited
The server log SHALL have a `bus audit:` line for every denied call, provide, emit and subscription (caller, token revocation id prefix, kind, method or subject, target, reason; for a per-call credential the caller and the credential's participant, `from=<caller> bearer=<participant>`) and for every control, mint, renew, revoke and key-rotation call, allowed or not; token bytes MUST NOT appear in the log.

**Verified by**: `bus_acl::denied_and_control_calls_are_audited`, `bus_acl::a_per_call_credential_is_a_bearer`.

#### Scenario: A denial
- **WHEN** `t-1` is denied `test.whoami`
- **THEN** the log has a line with `bus audit: deny`, `from=instance:t-1`, `method=test.whoami` and a reason, and no line contains the token
