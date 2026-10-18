## ADDED Requirements

### Requirement: The default Cloud is the production control plane
Without `--url` or `TILR_CLOUD`, `tilrd cloud login` SHALL use the production Cloud host that serves `/v1/enroll` and `/v1/machine/connect`.

#### Scenario: Login with defaults
- **WHEN** `tilrd cloud login` runs without `--url` and `TILR_CLOUD`
- **THEN** it posts to the production Cloud host's `/v1/enroll`
