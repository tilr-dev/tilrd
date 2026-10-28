## Design

Use the shared workflow outputs `tag`, `version`, `is_rc`; build only a nonempty tag. Stable publication additionally requires `is_rc=false` on a tag push. Exclude RC tag pushes to avoid recursion. Keep caller release jobs serialized with a queue distinct from the shared workflow queue.

The tag sets TILR_BUILD_VERSION. Before compilation a helper checks that the numeric tag version is at least the workspace Cargo version, leaving Cargo.toml and Cargo.lock unchanged. This avoids requiring version/changelog commits. Reuse the helper from every build checkout. Builds, signing, notarization and publication otherwise retain their dependencies and credentials.

Remove the upstream checker exemption: all pull requests into release branches must contain upstream patches. Runtime reload state, client reconnects, backpressure and protocol features are unaffected.
