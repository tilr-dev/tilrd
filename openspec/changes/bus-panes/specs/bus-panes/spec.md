## ADDED Requirements

### Requirement: tilrd provides pane methods on the bus
The server SHALL provide `pane.create`, `pane.list`, `pane.write`, `pane.screen`, `pane.resize` and `pane.close` as the reserved instance `tilrd`, always `up`, with JSON bodies; an unknown field SHALL be `bad_request` and an unknown pane `not_found` with `no such pane: <id>`.

**Verified by**: `bus_panes::tilrd_provides_pane_methods`.

#### Scenario: Methods listed
- **WHEN** the owner calls `bus.methods`
- **THEN** each `pane.*` method is listed with provider `tilrd`

#### Scenario: Unknown pane
- **WHEN** the owner calls `pane.screen {"pane_id": "<random uuid>"}`
- **THEN** the reply is `not_found` with `no such pane: <that uuid>`

### Requirement: A pane created on the bus is an ordinary pane
`pane.create {profile, command?, cwd?, cols?, rows?}` SHALL create the pane as `CreatePane` would, with the same profile rules and errors (as `bad_request` with the `create pane: …` message), and answer its `PaneInfo` as JSON; every tilrd client SHALL see it in `PaneList` and be able to subscribe to it.

**Verified by**: `bus_panes::a_pane_created_on_the_bus_is_an_ordinary_pane`.

#### Scenario: Created by an instance, watched by a client
- **WHEN** `t-1` calls `pane.create {"profile": "demo"}`
- **THEN** it gets a `pane_id`, every client gets a `PaneList` with that pane and `profile` `demo`, and a client that subscribes gets its frames

#### Scenario: Profile refused
- **WHEN** `t-1` calls `pane.create {"profile": "nope"}`
- **THEN** the reply is `bad_request` with `create pane: no such profile: nope`

### Requirement: Writing to a pane on the bus
`pane.write {pane_id, text | data_b64}` with exactly one of the two SHALL deliver the bytes to the pane's program in order and answer `{written}` once they reached the pty; a body over 1 MiB SHALL be `bad_request`.

**Verified by**: `bus_panes::writing_and_reading_a_pane_on_the_bus`.

#### Scenario: A command typed over the bus
- **WHEN** `t-1` writes `echo sum $((6*7))\r` to its pane
- **THEN** a client watching the pane sees `sum 42`

### Requirement: Reading a pane's screen on the bus
`pane.screen {pane_id, since_seq?, wait_ms?, history_lines?}` SHALL answer the pane's screen as text rows with `seq`, size, cursor, title, cwd, `alive` and `exit_code`, and up to `history_lines` (≤ 10000) scrollback lines; with `since_seq` it SHALL wait for a change after that `seq` or the program's exit, at most `min(wait_ms, 25 s)` and always before the call's deadline.

**Verified by**: `bus_panes::writing_and_reading_a_pane_on_the_bus`, `bus_panes::waiting_for_the_screen_to_change`.

#### Scenario: Screen after a command
- **WHEN** `t-1` wrote `echo hi\r` and calls `pane.screen` until a line equals `hi`
- **THEN** some answer has a line `hi` and the cursor on the line of the next prompt

#### Scenario: Wait for a change
- **WHEN** `t-1` calls `pane.screen {since_seq: S, wait_ms: 5000}` with the current `seq` S and 1 s later a client types into the pane
- **THEN** the answer comes after about 1 s with a `seq` greater than S

#### Scenario: Deadline shorter than the wait
- **WHEN** the call's `timeout_ms` is 500 and `wait_ms` is 5000 and nothing changes
- **THEN** the answer comes before 500 ms with `seq` S, not a `timeout`

### Requirement: Resizing and closing a pane on the bus
`pane.resize {pane_id, cols, rows}` SHALL resize the pane as `Resize` does; `pane.close {pane_id}` SHALL close it as `ClosePane` does and answer `{exit_code}` once the program ended.

**Verified by**: `bus_panes::resizing_and_closing_a_pane_on_the_bus`.

#### Scenario: Resize then close
- **WHEN** `t-1` resizes its pane to 100x30 and then closes it
- **THEN** `stty size` printed `30 100` before the close, the close answers an exit code, and every client gets `PaneExited`

### Requirement: Creating a pane on the bus needs pane rights
`pane.create` SHALL be `denied` unless the call's token has `pane_profile(<the profile>)` or `pane_profile_any()` (or, without a profile, `pane_unrestricted()`), and, when the request sets `command` or `cwd`, also `pane_override()`; the denial SHALL name the missing right and no pane SHALL start.

**Verified by**: `bus_panes::creating_a_pane_on_the_bus_needs_pane_rights`.

#### Scenario: Another profile
- **WHEN** `t-1` has `pane = ["profile:agent", "access:own"]` and calls `pane.create {"profile": "demo"}`
- **THEN** the reply is `denied` naming `pane_profile`, and no pane is listed

#### Scenario: No profile
- **WHEN** `t-1` with the same rights calls `pane.create {}`
- **THEN** the reply is `denied` naming `pane_unrestricted`

#### Scenario: Override
- **WHEN** `t-1` calls `pane.create {"profile": "agent", "command": ["/bin/sh"]}`
- **THEN** the reply is `denied` naming `pane_override`

### Requirement: Pane access follows the creator
With `pane_access("own")` an instance SHALL list, write, read, resize and close only panes whose `PaneInfo.created_by` is `bus:<its name>`; with `pane_access("any")` every pane; any other access MUST be `denied`. `created_by` SHALL be `owner` for panes created by tilrd clients and SHALL cross a reload.

**Verified by**: `bus_panes::pane_access_follows_the_creator`, `reload::a_bus_created_pane_keeps_its_creator_across_a_reload`.

#### Scenario: The owner's pane
- **WHEN** a client created a pane and `t-1` (`access:own`) calls `pane.screen` on it
- **THEN** the reply is `denied`, and `pane.list` from `t-1` does not include it

#### Scenario: Across a reload
- **WHEN** `t-1` created a pane, the server reloads, and `t-1` calls `pane.write` on it
- **THEN** the write succeeds and `ListPanes` shows `created_by` `bus:t-1`

### Requirement: A pane outlives the instance that made it
A pane created on the bus SHALL keep running when its creating instance stops, crashes or is reloaded; the instance's next process SHALL access it under the same rights.

**Verified by**: `bus_panes::a_pane_outlives_the_instance_that_made_it`.

#### Scenario: Instance reloaded
- **WHEN** `t-1` created a pane running `sleep 1000` and `tilr bus reload t-1` runs
- **THEN** the pane still runs and the new `t-1` process can call `pane.screen` on it
