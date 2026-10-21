# Proposal: No test servers left running after cargo test

## Why

A test process can disappear without running destructors. Its test servers and
copied executables then outlive it. Startup also has an unguarded interval before
readiness, and client cleanup depends on a command socket that tests can remove.

## What Changes

- Give every harness-owned server an independent ownership connection. Closing
  the connection, including process death, requests normal server shutdown.
- Establish ownership before readiness and retain it across daemon startup and
  executable reloads. Wait for owned process exit before removing test files.
- Exercise panic, signal and controlled timeout with confirmed readiness and OS
  exit events, including copied executables and unavailable command sockets.

## Capabilities

- Add test-server-lifetime requirements for harness-owned processes.

## Impact

The shared integration harness, client test machines and cleanup stand. The
ownership socket is an internal test hook; ordinary server startup is unchanged.
