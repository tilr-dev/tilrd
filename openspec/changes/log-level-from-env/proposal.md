# Proposal: RUST_LOG sets the server's log level

## Why

`RUST_LOG` is not honoured: the logger is initialised without an environment filter, so debug logging is impossible on a deployed server (Basic Memory `main/projects/tilr/tilr-server-na-pb.rexuni.dev-systemd-cloudflare-tunnel-tilr-pb.rexuni.dev`, `main/projects/tilr/tilr-probros-ssh-agenta-sshagent-most-mac-app-stranitsa-tilr-server-kak-ssh-a`). Still so at 1163abd: `tracing_subscriber::fmt()…init()` with no filter (`crates/tilrd/src/main.rs:257-261`), `tracing-subscriber = "0.3"` without the `env-filter` feature (`crates/tilrd/Cargo.toml:38`), and no `EnvFilter` or `RUST_LOG` anywhere in `crates/*/src`. The `tracing::debug!` lines that exist (e.g. `supervisor.rs:950`, `1048`, `1053`; `exec.rs:227`, `575`) can never be seen.

## What Changes

- `tilrd` (supervisor and pane processes, which run the same `main`) takes its log filter from `RUST_LOG` in tracing's directive syntax (`debug`, `tilrd=debug,info`); unset or empty means `info` (today's output); an invalid value means `info` plus one warning line `RUST_LOG ignored: <error>`.
- `RUST_LOG` stays in the server's environment like any other variable, so a reload (same environment) keeps the level and panes' shells see it as today.
- README "Running tilrd" documents it.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: "Foreground service with its log on stderr" names `RUST_LOG`.

## Non-goals

- A log-level protocol message or a config file key.
- Changing what is logged at which level.
- Logging in `tilr` (the client).

## Impact

`crates/tilrd/src/main.rs`, `crates/tilrd/Cargo.toml` (tracing-subscriber `env-filter` feature; `Cargo.lock`; `cargo deny check` for the added `regex`-family crates), `crates/tilrd/tests/common/mod.rs` (test servers start without the developer's `RUST_LOG`), a new test, README. No protocol change, no PROTOCOL_VERSION or feature change, no state format change. A reload keeps the environment, so the level carries over; an update from the previous release starts filtering at the reload into this one. Not breaking: without `RUST_LOG` the output is as before.
