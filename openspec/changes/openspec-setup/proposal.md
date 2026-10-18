# Proposal: OpenSpec for Claude Code and Codex

## Why

Set up the spec-driven workflow for both agents, as in tilr-dev/gitpm (its `openspec/changes/archive/2026-10-18-openspec-setup`), and migrate the existing design of tilrd into it (session of 2026-10-18).

## What Changes

- OpenSpec skills and `/opsx` commands for Claude Code (`.claude/`) and Codex (`.agents/`), `openspec/config.yaml` with the project context and rules.
- 18 capability specs (`openspec/specs/`) written against 13f9522 and brought in line with 47a8557 (release archive, installer, CI port); a retro archive (`openspec/changes/archive/`) of the work so far; open changes for the backlog.
- Spec coverage from the Dagger itests: each case in `itests/scenarios/` tagged with the requirements it verifies (`@case(spec=[...])`), `itests/tools/coverage.py` and `./dt coverage` writing `openspec/coverage/coverage.md` and `coverage.json` (offline), as in gitpm.

## Capabilities

### Modified Capabilities
- None (tooling/process only).

## Out of scope

- Any behavior change in `crates/`; the spec tags change no case's behavior.

## Impact

Uncommitted at the time of writing (branch `docs/openspec`). Archive this change with the commit that lands it ("Implemented by").
