# Tasks

## 1. Preserve report data

- [x] 1.1 Select `--report PATH`, the local latest report, or the existing output `coverage.json` in that order; verify `itests/tests/test_coverage.py` exercises both explicit and fallback paths and fails before the implementation.
- [x] 1.2 Preserve the existing last-run summary and statuses for current case IDs when no report exists; verify JSON and Markdown assertions in `itests/tests/test_coverage.py`.
- [x] 1.3 Document fallback and `--report PATH` in one README line; verify against this spec.

## 2. Verification and archive

- [ ] 2.1 Run the Python regression test and `./itests/dt coverage`; verify only this change's requirement/case lines change in generated coverage.
- [ ] 2.2 Run the applicable checks from AGENTS.md, record the commit in proposal.md, and archive this change; verify `openspec list` no longer shows it.
