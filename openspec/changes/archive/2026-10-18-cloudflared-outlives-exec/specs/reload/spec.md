## MODIFIED Requirements

### Requirement: cloudflared is stopped before the exec
Before its exec the supervisor SHALL stop its `cloudflared` (killed if it does not exit in time) so it never runs alongside the successor's; the successor SHALL start its own, and a reload that fails SHALL give the old supervisor its `cloudflared` back.

**Verified by**: `cloud::a_reload_replaces_cloudflared_with_one_of_the_new_server`, `cloud::a_cloudflared_ignoring_sigterm_does_not_hold_back_a_reload`.

#### Scenario: Linked server reloads
- **WHEN** a server with its tunnel up gets `SIGUSR1`
- **THEN** the new supervisor connects to Cloud again with a different `cloudflared` pid and the old `cloudflared` is gone

#### Scenario: cloudflared ignores SIGTERM
- **WHEN** a reload starts while the tunnel's `cloudflared` ignores SIGTERM
- **THEN** the old `cloudflared` is killed about 3 s later, the reload completes, and only the successor's `cloudflared` runs
