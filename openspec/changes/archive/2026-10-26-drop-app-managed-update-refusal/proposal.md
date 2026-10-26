## Why

The Mac app no longer owns a separate server copy or update channel (owner's decision, 2026-10-26). The legacy marker refusal in `tilrd update` prevents that installation from using the ordinary updater.

## What Changes

- Remove the refusal triggered by `app-managed.json` next to the executable.
- Exercise ordinary release installation with the legacy marker present.
- Remove the obsolete update requirement and document marker-independent updates.

## Non-goals

- Changing the release feed, archive format, reload protocol, or app installation.
- Deleting legacy marker files.

## Compatibility

No wire protocol, feature flag, or reload state changes. Updates from a previous release follow the existing installation and reload path; the marker is ignored by the new updater.
