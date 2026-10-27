## ADDED Requirements

### Requirement: Filesystem watch reconciliation
An active directory watch SHALL reconcile its direct entries after a source reports incomplete coverage and through periodic metadata checks, preserving connection ownership and existing change coalescing. Metadata-visible persistent changes SHALL be reported even if native path notifications are absent.

Notes: proposed behavior, with no implementation test in this research. Resource and latency policy is discussed in design.md.

#### Scenario: Native delivery is absent
- **WHEN** an immediate entry is modified persistently with changed metadata and no native path notification arrives
- **THEN** the next completed metadata reconciliation reports its change through FsChanged
- **AND** a subdirectory's contents are not reported as direct entries

#### Scenario: A source reports lost coverage
- **WHEN** a source reports lost or coarse coverage affecting an active directory watch
- **THEN** the daemon reconciles all affected watched direct entries and reports their differences
- **AND** repeated invalidations are retained until a reconciliation covering them completes

#### Scenario: A scan cannot read a directory
- **WHEN** a reconciliation cannot enumerate a watched directory because access is denied
- **THEN** unknown entries are not reported as removed merely because enumeration failed

#### Scenario: A stopped watch has an in-flight scan
- **WHEN** a watch is replaced, unwatched or disconnected before its reconciliation finishes
- **THEN** that reconciliation sends no notifications for the stopped registration

