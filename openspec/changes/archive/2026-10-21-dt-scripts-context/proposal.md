## Why

The dt source allowlist omits the release and Linux script fixtures and their
repository configuration. Rust tests compile but fail when they read those files
in the runtime container. The release fixtures also need the `shasum` command.

## What Changes

- Include scripts, release workflows, release configuration, tracked Cargo defaults
  and the Git ignore rules in the filtered source snapshot used by dt.
- Provide `shasum` in the runtime image so release fixtures can verify archives.
- Keep the release and Linux script regression tests enabled in dt.

## Impact

- Affected spec: `itest-port-stand`.
- Affected code: the Dagger module's source filter and runtime packages.
- Validation uses the existing `release` and `linux_script` Rust regression suites
  through dt, including a run against the original source filter.
