## MODIFIED Requirements

### Requirement: Server feature flags
`HelloAck.features` SHALL list exactly the capabilities this server can be run without and offers, in this order: `"clipboard"` (off: `--no-clipboard`), `"files"` (`--no-files` / `[files] enabled = false`), `"host-events"` (host-events socket not up), `"agent"` (`--no-agent` / `agent = false`), `"exec"` (`--no-exec` / `exec = false`), `"sandbox"` (the host cannot enforce sandboxed panes). What every server of the version has (`Ping`) MUST NOT be listed. A client MUST NOT rely on a message of a feature not listed.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `clipboard::the_clipboard_can_be_turned_off`, `files::files_can_be_turned_off`, `agent::the_agent_is_forwarded_unless_turned_off`, `exec::exec_is_offered_unless_turned_off` and `sandbox::sandbox_is_offered_where_it_can_be_enforced`.

#### Scenario: Everything on
- **WHEN** a server started with default options on a host that can enforce sandboxes is greeted
- **THEN** `HelloAck.features` is `["clipboard", "files", "host-events", "agent", "exec", "sandbox"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `"exec"` is missing from `HelloAck.features` and the rest are listed as before

#### Scenario: No sandbox on this host
- **WHEN** the server runs on Linux without Landlock ABI 4 (simulated by the failpoint `sandbox-unavailable`)
- **THEN** `"sandbox"` is missing from `HelloAck.features` and a sandboxed profile is refused as the `pane-profiles` spec says
