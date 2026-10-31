## Why
The server already reports its build in HelloAck, but clients cannot read the update channel configured in server.toml. The channel source is recorded in the existing update specification.

## What Changes
- Add the string update_channel to HelloAck, taken from server.toml (default stable).
- Print the channel on a separate line in tilr version, preserving the existing server version line for app consumers.
- Keep the protocol version and feature flags unchanged: older peers may ignore the added field.

## Reload and update
No state is persisted beyond server.toml. New connections report the channel loaded by the running server, including after restart or reload. Older servers omit the new field; the client does not infer a channel from an empty value.

## Out of scope
- Changing update selection, configuration, driver JSON output or application UI.
- Persisting a channel in the state directory.
