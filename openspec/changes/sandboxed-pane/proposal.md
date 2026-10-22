# Proposal: Sandboxed panes for agents

Deferred / optional: an optional feature, not scheduled now.

## Why

A pane in a sandbox for agents was proposed with open questions: Seatbelt per pane vs an App Sandbox signed pane binary on macOS (a quarantine experiment), Landlock or bubblewrap on Linux, per-pane loopback alias IPs (`127.10.x.x`), a tilrd-in-Docker mode, and a token that can only create sandboxed panes (Basic Memory `main/projects/tilr/tilr-ficha-sandboxed-pane-panel-v-pesochnitse-dlia-agentov`).

## Decision (2026-10-18)

- macOS: App Sandbox. The supervisor starts a separately signed pane binary that carries the sandbox entitlements (per the Basic Memory note: `com.apple.security.app-sandbox`, `network.client`, file exceptions for the allowed paths).
- Linux: Landlock, applied in the pane's `pre_exec` before the shell, inherited by all the shell's children, no root: file rules need kernel ≥ 5.13, TCP bind/connect rules ≥ 6.7.
- First step is an experiment on `com.apple.quarantine`: scripts and `cargo`-built binaries created inside an App Sandbox pane must run. If they cannot, the macOS approach is reconsidered.
- Rejected for now: Seatbelt `sandbox_init` profiles (deprecated API).
- Rejected for now: a tilrd-in-Docker mode (stays an idea, out of scope).

## What Changes

- Experiment first (design D1); nothing else until it passes.
- Then: `CreatePane` gains a sandbox request (allowed write paths, network on/off); the pane process applies it (macOS: runs the signed sandboxed pane binary; Linux: Landlock ruleset in `pre_exec`); `HelloAck.features` lists `sandbox` where the host supports it (it can be unavailable: old kernel, unsigned build), per the protocol version policy in `openspec/config.yaml`; version 1 remains unchanged before the first public release.

## Capabilities

### Modified Capabilities
- `panes`
- `access-control`

Spec deltas are not written yet (`skip_specs: true`): the mechanism depends on the experiment.

## Non-goals

- Seatbelt profiles, VMs, Endpoint Security / Network Extension.
- tilrd in Docker.
- Network isolation beyond loopback aliases; domain allowlists.
- A token that can only create sandboxed panes (later, with `read-only-viewer`'s token kinds).

## Impact

Additive `CreatePane` field and a `sandbox` feature; a second signed binary in the macOS app bundle (release and signing, outside this repo's Linux CDN); the sandbox request goes into the pane state so a reloaded pane keeps it (pane-state format bump). A reload execs the pane process in place: on macOS the sandboxed binary execs its own successor, on Linux the Landlock domain of the shell is unchanged by the pane's exec.
