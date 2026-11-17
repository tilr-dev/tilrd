## ADDED Requirements

### Requirement: The owner mints tokens
`tilr bus token --name NAME [--call P…] [--provide P…] [--emit P…] [--subscribe P…] [--control A[:INSTANCE]…] [--pane R…] [--origin ORIGIN] [--ttl DURATION]` SHALL call `bus.token` and print one token (base64url) on stdout; the token's participant SHALL be `token:NAME`; it SHALL fail for a ttl over 24 h.

**Verified by**: `client::tilr_bus_token_mints_a_token_that_works`.

#### Scenario: Mint and use
- **WHEN** the user runs `tilr bus token --name s --call test.echo --ttl 1h` and a call carries the printed token
- **THEN** `test.echo` is answered and `test.whoami` is `denied`

#### Scenario: Too long
- **WHEN** the user runs `tilr bus token --name s --call test.echo --ttl 48h`
- **THEN** it exits 1 and says the ttl is at most 24h
