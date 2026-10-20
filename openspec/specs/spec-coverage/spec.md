# spec-coverage Specification

## Purpose
Keep generated spec coverage reports available locally without storing run-specific artifacts in git.

## Requirements

### Requirement: Local coverage reports
The repository SHALL keep generated `openspec/coverage/` reports out of version control and ignore that directory. Developers SHALL generate reports on demand with `./itests/dt coverage`; instructions SHALL describe the reports as local artifacts without requiring them to be committed.

#### Scenario: Generate reports in a checkout
- **WHEN** a developer runs `./itests/dt coverage` with a valid case registry
- **THEN** the command exits 0 and writes `openspec/coverage/coverage.md` and `openspec/coverage/coverage.json` locally
- **AND** `git ls-files openspec/coverage` is empty and git ignores both generated files

### Requirement: Report selection

`./itests/dt coverage` SHALL accept `--report PATH` and use that report's run metadata and scenario results for the generated `coverage.json` and `coverage.md`. An explicit path SHALL take precedence over a local `itests/results/latest/report.json`; a missing explicit path SHALL fail visibly without replacing the existing coverage files. With no explicit path, the local latest report SHALL take precedence over previously generated coverage. Verified by `itests/tests/test_coverage.py`.

#### Scenario: Explicit report from another folder
- **WHEN** a caller supplies `--report PATH` for a valid report outside this checkout
- **THEN** the generated last-run summary and case statuses reflect that report

#### Scenario: Missing explicit report
- **WHEN** a caller supplies `--report PATH` that does not exist
- **THEN** coverage generation exits nonzero and does not overwrite either coverage file

#### Scenario: Local report
- **WHEN** no explicit report is supplied and the local latest report exists
- **THEN** its run metadata and scenario results are used

### Requirement: Preserve recorded run without a report

When neither an explicit nor a local report is available, `./itests/dt coverage` SHALL retain the last-run metadata and statuses of current case IDs from the existing `coverage.json` in its output directory. An existing `coverage.json` that cannot be read SHALL fail visibly without replacing the coverage files. It SHALL generate requirements and case lists from the current specs and registry. With no existing coverage file, it SHALL show no last run. Verified by `itests/tests/test_coverage.py`.

#### Scenario: No local report with existing coverage
- **WHEN** the local latest report is absent and an existing `coverage.json` records a run
- **THEN** regenerated JSON and Markdown retain that run and statuses of unchanged case IDs
- **AND** any changed requirements and cases follow the current specs and registry

#### Scenario: Unreadable previous coverage
- **WHEN** no report is available and the existing `coverage.json` cannot be read or lacks the recorded run fields
- **THEN** coverage generation exits nonzero with a message naming that file and does not overwrite either coverage file

#### Scenario: No recorded run
- **WHEN** no report or previous `coverage.json` exists
- **THEN** generated coverage shows no last run and no recorded case statuses
