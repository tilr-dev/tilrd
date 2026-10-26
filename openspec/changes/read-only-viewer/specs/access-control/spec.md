## ADDED Requirements

### Requirement: A viewer token makes a read-only connection
`tilrd token --viewer` (users: `tilr token --viewer`) SHALL print the viewer token, making `<state>/viewer-token` (64 hex characters, 0600) if there is none; `--new` SHALL replace it. A TCP `Hello` carrying the viewer token SHALL be accepted as read-only: `HelloAck.read_only` true and `HelloAck.features` empty. A server started with `--no-auth` SHALL have no viewer token and `tilr token --viewer` SHALL fail with `this server asks for no token`.

**Verified by**: `auth::the_viewer_token_makes_a_read_only_connection`.

#### Scenario: Greeted as a viewer
- **WHEN** `tilr token --viewer` has run and a TCP client sends `Hello` with its output
- **THEN** it receives `HelloAck` with `read_only` true and no features, and `<state>/viewer-token` is mode 0600

#### Scenario: No viewer token yet
- **WHEN** nobody ran `tilr token --viewer` and a client presents a random 64-character token
- **THEN** it receives `Error` "wrong access token"

### Requirement: A read-only connection watches and changes nothing
A read-only connection SHALL be served `ListPanes`, `Subscribe`, `Unsubscribe` and `Ping` as any client, and every other message from it MUST be answered `Error { message: "this connection is read-only" }` and have no effect: no byte reaches a pty, no pane is created, closed, resized or scrolled, no file, clipboard or exec is touched. It MUST NOT be sent `HostEvent`s, agent streams or clipboard writes.

**Verified by**: `auth::a_read_only_connection_sees_frames_and_changes_nothing` and itests UC-03B (`uc_03b.py::a_viewer_sees_the_pane_and_nothing_it_sends_reaches_it`).

#### Scenario: Typing as a viewer
- **WHEN** a viewer subscribed to a pane sends `KeyInput`, `Paste`, `RawInput`, `Resize` 10x5 and `Scroll` -3
- **THEN** it gets five `Error`s "this connection is read-only", the shell read nothing, and the pane's size and view are unchanged

#### Scenario: Frames like anyone
- **WHEN** a full client types `echo hi` into a pane a viewer watches
- **THEN** the viewer receives frames showing `hi`

#### Scenario: Files and exec
- **WHEN** a viewer sends `FsList ~` and `ExecStart ["true"]`
- **THEN** both are answered with `Error` "this connection is read-only" and no `FsEntries` or `ExecStarted` comes

### Requirement: Viewers are listed as read-only
`PaneInfo.viewed_by` SHALL list a read-only connection like any viewer, with `Viewer.read_only` true, and `PaneInfo.viewers` SHALL count it.

**Verified by**: `auth::a_read_only_connection_sees_frames_and_changes_nothing`.

#### Scenario: Two viewers
- **WHEN** a full client and a read-only one watch a pane
- **THEN** `ListPanes` shows `viewers` 2 and one `viewed_by` entry with `read_only` true

### Requirement: Replacing the viewer token cuts off its viewers
`tilr token --viewer --new` SHALL replace the viewer token and the running server SHALL close every read-only connection greeted with the old one, after `Error { message: "the viewer token was replaced" }`; the old token MUST then be refused.

**Verified by**: `auth::replacing_the_viewer_token_cuts_off_its_viewers`.

#### Scenario: Token replaced
- **WHEN** a viewer is connected and `tilr token --viewer --new` runs
- **THEN** the viewer receives that `Error` and its connection closes, and a `Hello` with the old token gets "wrong access token"
