# Proposal: Sandboxed panes for agents

Блокер: Which sandbox on macOS? Options: (a) Seatbelt (`sandbox_init` profile per pane) — flexible, deprecated API, no signing needed; (b) App Sandbox with a signed pane binary — official, rigid entitlements, needs the app's signing; (c) (a) by default and (b) as the "official" mode. Recommendation: (a) now, Linux via Landlock, revisit (b) if Seatbelt stops working.

## Why

A pane in a sandbox for agents was proposed with open questions: Seatbelt per pane vs an App Sandbox signed pane binary on macOS (a quarantine experiment), Landlock or bubblewrap on Linux, per-pane loopback alias IPs (`127.10.x.x`), a tilrd-in-Docker mode, and a token that can only create sandboxed panes (Basic Memory `main/projects/tilr/tilr-ficha-sandboxed-pane-panel-v-pesochnitse-dlia-agentov`).

## What Changes

- To be decided: `CreatePane` gains a sandbox profile; the pane process applies it before exec of the shell.

## Capabilities

### Modified Capabilities
- `panes`
- `access-control`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Network isolation beyond loopback aliases.

## Impact

Additive `CreatePane` field and feature flag; the profile must be in the pane state so a reloaded pane keeps it.
