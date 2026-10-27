# Proposal: Liveness by pongs, slow clients really dropped, client_id supersedes

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-041 and UC-049 were red (see `2026-10-17-itests-bad-network`); a client back on a new address left a ghost viewer until the timeout. Decisions: no resume windows; input sent into a broken connection may be lost but never duplicated; the p50 delay of 1-13 s under 30% loss (UC-043) is accepted as TCP retransmits and QUIC is deferred (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`, `main/projects/tilr/tilrd-idei-read-only-zritel-client-id`).

## What Changes

- The server judges a client by the pongs to its own pings (silent vs deaf), closes with 4002 `timeout`, and resets the socket (SO_LINGER 0) when the close cannot get out.
- A lagging client skips diffs and gets the latest full frame (`outbox.rs`, 250 ms lag, `TCP_NOTSENT_LOWAT` 4 KiB), its own input still echoed by `last_input_seq`; a dropped slow client is really closed (4001 `too slow`).
- `Hello.client_id` (UUID v4 per client instance): the same id and token supersede a live connection at once (4000 `superseded`) before the new `HelloAck`.
- `PaneInfo.viewed_by`; metrics count drops by reason and skipped frames.
- The files test no longer races its subscription; the test client's deadline is not reset by pings; itests UC-052 and stricter UC-042/044.

## Capabilities

### Modified Capabilities
- `client-liveness`
- `metrics`
- `panes`

## Out of scope

- Feature flags for `client_id` and close codes (open change `protocol-feature-flags`).
- A read-only viewer role (open change `read-only-viewer`).

## Impact

Implemented by: [`ff3ab84`](https://github.com/tilr-dev/tilrd/commit/ff3ab84) A client that no longer hears the server is dropped, a slow one really is, and a client back on a new connection supersedes its old one … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
