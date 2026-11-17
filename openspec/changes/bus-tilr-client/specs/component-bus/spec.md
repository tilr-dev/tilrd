## ADDED Requirements

### Requirement: tilr bus
`tilr bus call METHOD [PAYLOAD | - | --file PATH] [--target NAME] [--timeout DURATION] [--content-type TYPE] [--credential TOKEN]` SHALL make the call (`content_type` `application/json` by default) and write the reply's payload to stdout unchanged and exit 0, or write `<code>: <message>` to stderr and exit 1; `tilr bus ls`, `start`, `stop`, `reload` and `reload-config` SHALL call the matching `bus.*` method and print its result.

**Verified by**: `client::tilr_bus_call_prints_the_reply`.

#### Scenario: Echo from stdin
- **WHEN** the user runs `printf 'a\0b' | tilr bus call test.echo -`
- **THEN** stdout is the 3 bytes `a`, NUL, `b` and the exit status is 0

#### Scenario: Missing method
- **WHEN** the user runs `tilr bus call nobody.here`
- **THEN** stderr starts with `not_found: ` and the exit status is 1

### Requirement: tilr bus sub
`tilr bus sub PATTERN` SHALL subscribe to `PATTERN` and print one line per event (subject, a space, the payload as JSON for the core's events, else base64) until interrupted.

**Verified by**: `client::tilr_bus_sub_prints_events`.

#### Scenario: Watching events
- **WHEN** the user runs `tilr bus sub 'pane.*'` and a pane is created
- **THEN** a line starting with `pane.created ` is printed
