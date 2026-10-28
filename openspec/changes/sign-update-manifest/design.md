# Signed update metadata

Use raw Ed25519 with ring (already used transitively for TLS). A single JSON envelope contains `payload` (a JSON string) and `signature` (128 hexadecimal characters). The signature covers the exact UTF-8 payload bytes. The payload contains `version`, `platform`, `archive`, and `sha256`. Reject unexpected archive names, invalid versions, wrong platforms and malformed hashes. Metadata is limited to 16 KiB.

Publish `releases/<platform>/latest.manifest.json` or `beta.manifest.json` next to the unchanged legacy pointer. The manifest version must match the pointer. Prepare every selected manifest before uploading anything; upload archives and checksums before signed metadata, then legacy pointers. A racing pointer/manifest publication can fail closed and be tried again by the user. Rollbacks remain supported; signatures do not prevent replay of an earlier signed release.

The public key is embedded from `TILR_UPDATE_PUBLIC_KEY` (64 hex characters) at build time, never taken from the updater environment. Debug builds use a public fixture key; release builds require an explicit non-fixture key. Build jobs receive the repository public-key variable, while only the CDN publication step receives the signing secret. OpenSSL signs the exact payload, using a temporary private-key file removed on exit. Document key generation and environment setup; no production private key enters the repository.

Legacy `.sha256` and unsigned channel pointers remain for existing clients and the installer. New updates never fall back to their checksum. Removing these legacy objects follows a signed release separately.
