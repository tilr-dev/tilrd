# Tasks

## 1. Setup

- [x] 1.1 OpenSpec skills and commands installed for Claude Code and Codex
- [x] 1.2 Design migrated into specs, archive and open changes; verify `openspec validate --all --strict`
- [x] 1.3 Specs brought in line with 47a8557; verify every `**Verified by**` citation names an existing test or case
- [x] 1.4 itests cases tagged and `./dt coverage` writes `openspec/coverage/`; verify `uv run --no-project python itests/tools/coverage.py` exits 0

## 2. Land

- [x] 2.1 Commit on the owner's word and archive this change with that commit; verify `openspec list` no longer shows it
