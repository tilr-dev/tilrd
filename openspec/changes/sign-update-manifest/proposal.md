# Sign update manifests

## Why
A checksum served by the same CDN as an archive does not authenticate a release when CDN objects are replaced.

## What Changes
- Require an Ed25519-signed manifest before downloading an update, with no unsigned fallback.
- Publish signed stable and beta metadata in the CDN job while retaining legacy pointers and checksum files.
- Embed the verification key at build time and document provisioning the CDN signing secret.

## Impact
The updater, release publication, local CDN fixtures and updating documentation change together.
