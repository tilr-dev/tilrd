# Notarize macOS releases with an App Store Connect API key

## Why
CI notarization used an Apple ID with an app-specific password. An App Store Connect API key is not tied to Dan's personal Apple ID, needs no 2FA and is revoked on its own (Dan, 2026-10-26).

## What Changes
- `sign-macos` stores the notarytool profile from `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID` and `APPLE_API_ISSUER_ID`; `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_NOTARY_PASSWORD` are gone.
- The `.p8` file lives under RUNNER_TEMP only until stored, and is removed again on failure.

## Impact
- Specs: release-process, «macOS releases require Developer ID and accepted notarization».
- README lists the new secrets.
