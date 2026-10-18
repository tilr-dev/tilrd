# Design

## Context

At 1163abd the host socket (`crates/tilrd/src/host.rs`) reads JSON lines from programs in panes, checks the peer uid, hands each request to the supervisor (`deliver`) and writes one reply line per request. The supervisor sends `HostEvent` to the pane's subscribed clients with `"host-events"` and answers with how many it reached. Files have roots (`[files] roots`, `crates/tilrd/src/files.rs`), checked for every `Fs*` request.

## Decisions

### D1. Event ids are random, not counters
`HostEvent.event_id` is a random u64 (from the OS RNG), shown to the waiting program as a decimal string. Random ids need no state across a reload: a waiter can re-register with the new supervisor (D3), and a client's `HostEventDone` sent to the new supervisor still finds it. Rejected: a counter (would need handing over to stay unique).

### D2. Who is waited on
With `"wait": true` the event goes as today to every subscribed client with `"host-events"`, but the request fails before sending anything when none of them lists `"host-events-wait"` (a git commit must not go on with an unedited message). The first `HostEventDone` with that id from a client the event was sent to ends the wait; done messages from other clients, or for ids nobody waits on, are ignored. A client that disconnects does not end the wait: it may reconnect (network loss, a reload) and still send its done.

### D3. Waiting across a reload
The waiting `tilr` keeps its host socket connection open; the supervisor keeps `event_id → connection`. A reload closes the connection; `tilr` then reconnects to `$TILR_SOCK` (retrying for up to 10 s, as clients do after a reload) and sends `{"name":"wait","pane_id":...,"event_id":"<id>"}`; the new supervisor registers it. A done that arrives while nobody waits on the id (between the exec and the re-registration) is kept for 30 s, keyed by id, so the re-registering waiter gets it. No state file changes.

### D4. Roots check on the host socket
For kind `file` with the files feature on, the supervisor resolves the path as `FsStat` would and refuses one outside every root with the same message as the files protocol. With files off the event is delivered as today (a client may open it some other way).

## Reload and reconnect

See D3: nothing crosses the reload; ids are random; the waiter re-registers; a done that comes early is held 30 s. A client that reconnects keeps its open editors and sends `HostEventDone` when the user closes one.

## Backpressure

`HostEventDone` and the reply lines are small and one per edited file; held dones expire after 30 s and are capped at 1024 (oldest dropped).
