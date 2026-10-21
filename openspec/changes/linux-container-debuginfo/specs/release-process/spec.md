## ADDED Requirements

### Requirement: The Linux dev container builds with line-tables-only debuginfo
`scripts/linux.sh` SHALL pass `CARGO_PROFILE_DEV_DEBUG=line-tables-only` into its container, so the dev and test profile built there carries the same debuginfo as on a host whose cargo configuration sets `[profile.dev] debug = "line-tables-only"`, which the container does not see. Release builds SHALL be unaffected.

**Verified by**: `release::the_linux_dev_container_builds_with_line_tables_only_debuginfo` (static: `scripts/linux.sh`).

#### Scenario: Tests in the container
- **WHEN** `scripts/linux.sh` runs `cargo test` or `cargo build` in the dev profile
- **THEN** cargo in the container sees `CARGO_PROFILE_DEV_DEBUG=line-tables-only` and builds `target/debug/tilrd` with line tables only, not full debuginfo
