## ADDED Requirements

### Requirement: Local coverage reports
The repository SHALL keep generated `openspec/coverage/` reports out of version control and ignore that directory. Developers SHALL generate reports on demand with `./itests/dt coverage`; instructions SHALL describe the reports as local artifacts without requiring them to be committed.

#### Scenario: Generate reports in a checkout
- **WHEN** a developer runs `./itests/dt coverage` with a valid case registry
- **THEN** the command exits 0 and writes `openspec/coverage/coverage.md` and `openspec/coverage/coverage.json` locally
- **AND** `git ls-files openspec/coverage` is empty and git ignores both generated files
