## 1. Specification and guidance

- [x] 1.1 Rename the mandatory contract and update version-check scenarios; verify with strict OpenSpec validation.
- [x] 1.2 Document the pre-release freeze and later release-step policy in the protocol Notes, config, README, schema header and repository guidance; verify by reviewing these texts together.

## 2. Shared version and integration coverage

- [x] 2.1 Set the shared version to 1 and align USE_CASES.md; verify with `panes::a_client_built_for_another_protocol_version_is_turned_away` covering rejected versions 0 and 2 and a working version 1 client.
- [x] 2.2 Run the repository checks and validate spec coverage with `./itests/dt coverage`.
