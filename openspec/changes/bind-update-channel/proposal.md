# Bind update manifests to their channel

## Why
A CDN can replay a correctly signed beta manifest through the stable pointer because signed metadata does not identify its channel.

## What Changes
- Bind stable or beta to the signed payload and reject metadata from another channel before requesting an archive.
- Update production and test signers and document channel binding.
- Preserve intentional rollbacks and the known replay limitation within a channel.

## Impact
The updater, manifest signers, integration fixtures and release documentation change together. The first signed release must understand the channel field.
