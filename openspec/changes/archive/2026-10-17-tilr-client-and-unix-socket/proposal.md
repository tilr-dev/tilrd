# Proposal: The tilr client; a Unix socket for the local daemon

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-010 was red since stage 1: there was no `tilr` that starts the server. Decisions: `tilrd` is a service and never run by hand; all user commands go through `tilr`; one daemon per socket (Basic Memory `main/projects/tilr/tilrd-naming-install-and-run-model`, `main/projects/tilr/tilrd-idei-read-only-zritel-client-id`, `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- `crates/tilr`: `new`, `attach`, `ls`, `kill`, `kill-server`, `ps`, `version`, `token`, `cloud`, `update`, `edit`, `open`; `client.toml`; attach in raw mode with Ctrl+] d to detach and a disconnected line while it reconnects with the same `client_id`.
- `tilr` starts `tilrd` when none runs (double fork, setsid, a log file), holding a flock lock it hands to the daemon (`--lock-fd`).
- The Unix socket in the runtime directory (0700 directory, 0600 socket, the peer's uid checked, no token); an abandoned socket is replaced, a foreign one left with an error.
- User commands run inside the daemon (`ExecStart.server_command`, Unix socket only); a reload hands the Unix listener and its lock to the new image.
- itests group B, UC-034, UC-037.

## Capabilities

### Modified Capabilities
- `tilr-cli`
- `daemon-lifecycle`
- `access-control`
- `exec`
- `reload`

## Out of scope

- Checking the peer of the socket from the client side (open change `client-socket-peer-check`).
- Robust attach on errors and signals (open change `attach-robustness`).

## Impact

Implemented by: [`87514c6`](https://github.com/tilr-dev/tilrd/commit/87514c6) The client is tilr … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
