# Design: Remove tilr.v1 and the tilr-proto crate

Shared design: `openspec/design/unified-bus.md` §B3 (refusal packet), §B14, §T1, §T2, O2, O3,
§M2, §M3. This change's part follows.

## Decisions

- **11.2 lives here.** The main specs are rewritten in bus terms by the change after which the
  `tilr.v1` names stop being true; the requirements already modified by other changes are not
  modified again (O3 lists them). A separate change without code was rejected: it would hold no
  delta until 11.2 is written (so `openspec validate --strict` fails without `skip_specs`), and
  its text would be true only after this change anyway.
- **The four protocol tests** (`panes::a_malformed_packet_gets_an_error_and_the_connection_stays`,
  `panes::a_message_type_the_server_does_not_know_is_ignored`,
  `panes::a_client_that_skips_hello_is_turned_away`,
  `panes::a_client_built_for_another_protocol_version_is_turned_away`) move from
  `tests/common/v1.rs` to the bus here, and `v1.rs` is deleted.
- **Release notes.** GitHub generates stable notes (AGENTS.md); the M3 paragraph is written in
  README ("Updating") and in the PR description for the owner to put into the notes when the new
  minor is released.

## Reload and reconnect

No new state. After this change a `tilr.v1` client gets the refusal and is closed on every
connection, also after a reload.

## Backpressure

None new.
