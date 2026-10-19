## ADDED Requirements

### Requirement: Report selection

`./itests/dt coverage` SHALL accept `--report PATH` and use that report's run metadata and scenario results for the generated `coverage.json` and `coverage.md`. An explicit path SHALL take precedence over a local `itests/results/latest/report.json`; a missing explicit path SHALL fail visibly without replacing the existing coverage files. With no explicit path, the local latest report SHALL take precedence over previously generated coverage. Verified by `itests/tests/test_coverage.py`.

#### Scenario: Explicit report from another worktree
- **WHEN** a caller supplies `--report PATH` for a valid report outside this worktree
- **THEN** the generated last-run summary and case statuses reflect that report

#### Scenario: Missing explicit report
- **WHEN** a caller supplies `--report PATH` that does not exist
- **THEN** coverage generation exits nonzero and does not overwrite either coverage file

#### Scenario: Local report
- **WHEN** no explicit report is supplied and the local latest report exists
- **THEN** its run metadata and scenario results are used

### Requirement: Preserve recorded run without a report

When neither an explicit nor a local report is available, `./itests/dt coverage` SHALL retain the last-run metadata and statuses of current case IDs from the existing `coverage.json` in its output directory. It SHALL generate requirements and case lists from the current specs and registry. With no existing coverage file, it SHALL show no last run. Verified by `itests/tests/test_coverage.py`.

#### Scenario: Fresh worktree with existing coverage
- **WHEN** the local latest report is absent and an existing `coverage.json` records a run
- **THEN** regenerated JSON and Markdown retain that run and statuses of unchanged case IDs
- **AND** any changed requirements and cases follow the current specs and registry

#### Scenario: No recorded run
- **WHEN** no report or previous `coverage.json` exists
- **THEN** generated coverage shows no last run and no recorded case statuses
