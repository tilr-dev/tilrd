## Decisions
Use the existing versioned archive layout for both channels. A release script --channel stable|beta option selects the pointer; stable remains the default and maps to latest. Beta skips installer publication. The workflow passes beta for candidates and stable for releases. Stable does not move beta: stable promotion follows beta testing.

The updater accepts a typed channel flag only on tilrd update. Fetch-release, tilr update and the installer retain their stable behavior. A missing beta pointer fails normally, without falling back to stable.

## Validation
Exercise publication with the existing stub AWS harness and update with a local HTTP CDN. Assert upload order, pointer isolation, installer isolation, stable default and explicit stable, beta installation, invalid channels and missing beta.
