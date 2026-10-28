## ADDED Requirements

### Requirement: Stable releases propose the desktop server pin
After stable GitHub assets are uploaded, the release workflow SHALL use tilr-bot to propose the exact version and released protocol in `tilr-dev/desktop-mac` main through one pin pull request. It SHALL preserve the pin's positive artifact revision, run ordinary desktop CI and require ordinary review and merge. Candidates SHALL NOT propose updates.

#### Scenario: Stable release
- **WHEN** GitHub assets for stable `vX.Y.Z` have been uploaded after `release_created` is true
- **THEN** tilr-bot opens the pin update pull request into desktop-mac main
- **AND** its title is `chore: bump tilrd to X.Y.Z`
- **AND** `server-release.json` selects `X.Y.Z` and `PROTOCOL_VERSION` from the released source, preserving `build` from main
- **AND** ordinary desktop CI acquires that exact release through `fetch_server.py`

#### Scenario: Candidate or no release
- **WHEN** a release candidate is built or no stable release was created
- **THEN** no desktop pin pull request or branch is created or updated

#### Scenario: An update pull request is already open
- **WHEN** another stable release is published while the bot pin pull request is open
- **THEN** the same pull request is updated from current main with the new pin, title and description
- **AND** no second pull request is opened

#### Scenario: Missing or invalid release metadata
- **WHEN** the released protocol metadata is unavailable or invalid
- **THEN** the update fails without publishing a guessed pin

#### Scenario: Main already pins the release
- **WHEN** main already has the same version and protocol
- **THEN** no formatting-only change or new pull request is published
