# Proposal: PaneActivity: tell clients that a detached pane did something

Блокер: What counts as pane activity? Options: (a) bell only; (b) output after N seconds of silence; (c) the foreground process finishing (needs the pty's foreground pgid); (d) title change. Recommendation: (a)+(c) first — clear signals for "command done" and "attention", little noise; (b) as an opt-in per pane.

## Why

A lightweight event from a pane nobody watches was requested on 2026-10-03; what counts as an "event" is open: output after silence, the foreground process finishing, a bell, a title change (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`). A related idea is events, wake-up hooks and timers for agents (`main/projects/tilr/tilr-ideia-sobytiia-mcp-khuki-probuzhdeniia-agentov-i-taimery`).

## What Changes

- To be decided: which triggers, sent as a `PaneActivity{pane_id, kind}` message to clients with a feature flag, rate-limited per pane.

## Capabilities

### Modified Capabilities
- `panes`
- `protocol`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Push notifications to devices (client side).
- Agent hooks and timers (a later change on top of this one).

## Impact

Additive proto message and feature flag (`pane-activity`); per-pane trigger state must survive a reload (in the pane state; bump its format).
