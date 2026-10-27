## MODIFIED Requirements

### Requirement: macOS releases require Developer ID and accepted notarization
The macOS release helper SHALL require macOS and Apple signing credentials, sign and verify both executables with Developer ID, hardened runtime and a secure timestamp, and require Accepted notarization before producing distributable artifacts. CI SHALL use a temporary keychain and remove it even on failure.

#### Scenario: Signed bytes are published
- **WHEN** both macOS executables are signed and notarization is Accepted
- **THEN** the CDN and GitHub archives contain those exact signed executable bytes and checksums describe the completed archives

#### Scenario: Missing Apple credentials
- **WHEN** a manual macOS release helper invocation lacks its identity or notary profile
- **THEN** it fails before building or publishing

#### Scenario: Upload previously completed macOS builds
- **WHEN** upload-only receives completed macOS CDN layouts
- **THEN** it checks their versions and checksums and uploads them without requiring Apple credentials

#### Scenario: Signing and notarization configuration
- **WHEN** a macOS release is signed
- **THEN** a Developer ID Application identity and a notarytool keychain profile are required before signing
- **AND** both signed executables are submitted together in a ZIP to Apple

#### Scenario: Signing or notarization fails
- **WHEN** signing or signature verification fails, or notarization fails, returns malformed output or a status other than Accepted
- **THEN** no distributable archive or new latest pointer is produced or published

#### Scenario: CI imports Apple credentials
- **WHEN** CI releases macOS binaries
- **THEN** the build job has no environment or Apple secrets and uploads unsigned binaries with SHA-256 checksums
- **AND** a separate clean sign-macos runner downloads and verifies those checksums before importing Apple credentials from the apple-signing environment
- **AND** the signing job performs no checkout or build, imports a non-exportable private key, signs and notarizes the binaries, and uploads signed artifacts only after Accepted notarization
- **AND** notarization uses an App Store Connect API key from the apple-signing environment, not an Apple ID password; its key file is deleted right after it is stored in the keychain and again on failure
- **AND** the keychain is removed even on failure, without exporting its path, identity or notary profile through GITHUB_ENV
- **AND** a separate packaging job consumes only signed macOS artifacts for GitHub and CDN publication
