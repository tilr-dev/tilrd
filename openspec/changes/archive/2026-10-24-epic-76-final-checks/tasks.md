## 1. Coverage evidence

- [x] 1.1 Add the named Python test citation contract and requirement-local Verified by lines; verify with `openspec validate --all --strict`.
- [x] 1.2 Recognize Python citations separately from Cargo citations; verify JSON and Markdown with `itests/tests/test_coverage.py`, including a regression that fails before the fix.
- [x] 1.3 Verify the cited tests with `itests/tests/test_port_stand.py` and `scripts/test_server_cleanup_stand_selftest.py`; test local report generation and git ignore behavior in `itests/tests/test_coverage.py`.

## 2. Final validation

- [x] 2.1 Run UC-007 and UC-043 through `./itests/dt`; regenerate local reports with `./itests/dt coverage` and inspect uncovered requirements.
- [x] 2.2 Run the checks from AGENTS.md via the finish workflow, including macOS and Linux, formatting, Clippy with and without failpoints, dependency and secret checks, and strict OpenSpec validation.
- [x] 2.3 Record the implementation commit and archive this change.
