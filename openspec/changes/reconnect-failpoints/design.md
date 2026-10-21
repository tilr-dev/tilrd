# Design: Reconnect test controls

- Hooks: `reconnect-client-accepted`, `reconnect-unwatched-output:<pane id>`,
  `reconnect-subscribe:<pane id>`, `reconnect-full-frame:<pane id>` and
  `reconnect-client-lagging:<pane id>`. The full-frame hook precedes enqueueing
  for each viewer; it does not claim the peer has received the frame.
- Pane hooks hold supervisor event processing while socket tasks keep running.
- The read-pause handle's disappearance is checked by its channel error path;
  it has no separate integration scenario.

