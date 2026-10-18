# Proposal: RUST_LOG sets the server's log level

## Why

`RUST_LOG` is not honoured: the logger is initialised without an environment filter, so debug logging is impossible on a deployed server (Basic Memory `main/projects/tilr/tilr-server-na-pb.rexuni.dev-systemd-cloudflare-tunnel-tilr-pb.rexuni.dev`, `main/projects/tilr/tilr-probros-ssh-agenta-sshagent-most-mac-app-stranitsa-tilr-server-kak-ssh-a`). Verified at 13f9522: no `EnvFilter` or `RUST_LOG` in `crates/*/src`.

## What Changes

- The server reads `RUST_LOG` (default `info`) through tracing's environment filter; pane processes inherit it.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Out of scope

- A log-level protocol message.

## Impact

`crates/tilrd/src/main.rs`, `Cargo.toml` (tracing-subscriber `env-filter` feature; `cargo deny check`). A reload keeps the environment, so the level carries over. No protocol change.
