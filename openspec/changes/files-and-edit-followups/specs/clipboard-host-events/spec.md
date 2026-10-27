## ADDED Requirements

### Requirement: tilr edit --wait returns when the file is closed
`tilr edit --wait FILE` and `tilr open --wait TARGET` SHALL send the request with `"wait": true` and SHALL exit only when a client the event went to sends `HostEventDone` with its `HostEvent.event_id`: 0 when its `error` is empty, 1 printing the error otherwise. A request with `"wait": true` SHALL be refused with `no attached client can wait for the file`, and sent to no client, when no receiving client lists `"host-events-wait"` in `Hello.features`.

**Verified by**: `files::tilr_edit_wait_returns_when_the_client_is_done`, `files::tilr_edit_wait_needs_a_client_that_can_wait` and itests UC-076 (`uc_076.py::git_commit_with_tilr_edit_wait_as_the_editor_takes_the_message_from_the_client`).

#### Scenario: Waiting for the client
- **WHEN** a client with `"host-events"` and `"host-events-wait"` watches a pane and `tilr edit --wait notes.md` runs in it
- **THEN** the client receives a `HostEvent` with a non-zero `event_id` and `tilr` keeps running until the client sends `HostEventDone` with that id, then exits 0

#### Scenario: Nobody can wait
- **WHEN** the only client watching the pane lists `"host-events"` but not `"host-events-wait"`
- **THEN** `tilr edit --wait notes.md` exits 1 with `no attached client can wait for the file` and the client receives nothing

#### Scenario: As the git editor
- **WHEN** `GIT_EDITOR="tilr edit --wait" git commit` runs in a pane and the client writes the message file and sends `HostEventDone`
- **THEN** the commit is made with that message

### Requirement: Waiting goes on across a reload
A `tilr edit --wait` whose connection to the host socket is lost SHALL reconnect within 10 s and wait on the same `event_id`; a `HostEventDone` that reaches the server while nobody waits on its id SHALL be kept for 30 s for a waiter that comes back.

**Verified by**: `reload::tilr_edit_wait_goes_on_across_a_reload`.

#### Scenario: Reload while waiting
- **WHEN** `tilr edit --wait notes.md` is waiting, the server reloads, the client reconnects and sends `HostEventDone` for the event
- **THEN** `tilr` exits 0

### Requirement: tilr edit outside the shared directories is refused at once
With files on, a `tilr edit` (kind `file`) whose path leads outside every `[files] roots` directory SHALL be refused on the host socket with `<path>: outside the directories this server shares` and SHALL reach no client.

**Verified by**: `files::tilr_edit_outside_the_shared_directories_is_refused_at_once`.

#### Scenario: A file in /tmp
- **WHEN** roots are `["~"]`, a client watches the pane and `tilr edit /tmp/x` runs in it
- **THEN** `tilr` exits 1 printing `/tmp/x: outside the directories this server shares`, and the client receives no `HostEvent`

#### Scenario: Files turned off
- **WHEN** the server runs with `--no-files` and `tilr edit /tmp/x` runs in a watched pane
- **THEN** the client receives the `HostEvent` as before
