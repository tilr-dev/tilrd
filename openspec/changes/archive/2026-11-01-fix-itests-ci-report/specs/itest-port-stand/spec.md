## ADDED Requirements

### Requirement: Complete integration report export
The integration stand SHALL export the report after all selected cases finish,
including complete captured logs, report JSON, JUnit XML, a text summary and the
aggregate status, when the combined case logs are large. Report construction SHALL
preserve each case's result and log contents.
Captured log contents SHALL travel as files rather than GraphQL arguments or span
attributes. The exported directory SHALL remain valid after temporary report
source files are removed.

#### Scenario: Large aggregate logs
- **GIVEN** hundreds of completed cases with large captured logs
- **WHEN** the stand exports their report
- **THEN** all captured logs and report files are available in the exported directory
- **AND** the summary counts and aggregate status match the case results

### Requirement: Visible report export failures
The dt test wrapper SHALL preserve Dagger's diagnostic output and return a nonzero
exit status when Dagger fails to export a report.

#### Scenario: Dagger writes its error to stdout
- **WHEN** Dagger writes a diagnostic to stdout and exits nonzero during report export
- **THEN** the diagnostic is visible in the calling job's log
- **AND** dt exits nonzero without claiming a completed report
