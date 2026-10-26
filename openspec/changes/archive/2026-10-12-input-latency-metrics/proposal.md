# Proposal: Input latency through frames; Prometheus /metrics

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Measuring the terminal chain end to end needed the server's part of the latency and per-pane counters (Basic Memory `main/projects/tilr/tilr-izmerenie-tsepochki-terminala-end-to-end-tz-2026-10-12`).

## What Changes

- A client names its input by `ClientMessage.seq` (`HelloAck.seq_base + n`, `seq_base = client_id << 32`); every frame carries `Frame.last_input_seq` and `FrameTiming`; input that changed nothing gets an empty frame.
- Key, mouse, paste and raw input is credited once the pty took its last byte; Scroll and Resize when applied.
- Connection ids carry across a reload (`SupervisorState.next_client_id`); pending input and `last_input_seq` go to the reloaded pane (pane state format 3).
- `GET /metrics` in the Prometheus text format behind the token, or without one for this machine only (loopback or listen-address Host, `Sec-Fetch-Site: cross-site` refused; incremental head scan, 5 s response timeout).
- The test harness unsets `TILR_CLIPBOARD_DIR`, so the suite passes inside a Tilr pane.

## Capabilities

### Modified Capabilities
- `metrics`
- `reload`

## Out of scope

- Nothing recorded.

## Impact

Implemented by: [`79397e2`](https://github.com/tilr-dev/tilrd/commit/79397e2) Input latency and metrics: a client names its input by ClientMessage.seq … (2026-10-12). This commit is the merge base of `main` and `itests`.
