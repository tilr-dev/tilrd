## ADDED Requirements

### Requirement: Specification change authorization on dev
After the trusted workflow is added to main and the dev ruleset is activated, pull requests into dev SHALL pass the required spec-change check. One shared checker SHALL enforce intake Spec scope and accept authorization only from Dan's comments, using the dedicated read-only task credential. The check SHALL run only from the trusted pull_request_target workflow in main; the dev tree SHALL carry no copy of that workflow. Other tests SHALL remain local for dev.

#### Scenario: Existing specification is changed without authorization
- **WHEN** an intake-ready comment declares Spec scope and a pull request removes or changes existing specification lines without a matching spec-change entry from Dan
- **THEN** the check fails and lists the uncovered requirements

#### Scenario: Additions and the task's own change
- **WHEN** a pull request only adds requirements or edits its own change declared by intake
- **THEN** the check passes without an authorization block
- **AND** deltas that modify existing requirements still require authorization

#### Scenario: Authorized changes
- **WHEN** every affected existing requirement and foreign change has a matching operation in Dan's spec-change block for this component
- **THEN** the check passes

#### Scenario: Bot impersonates authorization
- **WHEN** an authorization block is posted by a bot instead of Dan
- **THEN** the block does not authorize any changes

#### Scenario: Task predates scope declarations
- **WHEN** the task's intake-ready comments contain no Spec scope line
- **THEN** the check passes as a legacy task

#### Scenario: Foreign change edited or archived
- **WHEN** a pull request edits or archives another task's change
- **THEN** the matching edit or archive entry from Dan is required

#### Scenario: Task data is unavailable
- **WHEN** the task cannot be identified or its comments cannot be read completely
- **THEN** the check fails with an actionable error

#### Scenario: Local merge automation
- **WHEN** local merge automation invokes the checker with a repository, base, head, component, and task branch
- **THEN** the same authorization rules and exit status apply as in CI

#### Scenario: Candidate code cannot replace the checker
- **WHEN** a pull request changes checker scripts or workflow definitions
- **THEN** the check still runs the workflow from main, not a workflow file from the pull request or dev
- **AND** the trusted workflow reads candidate Git objects without checking out or executing candidate files
- **AND** the shared checker content is verified against a digest pinned in main's workflow before execution, and a new digest takes effect only through a pull request into main
- **AND** only the checker process receives the read-only task credential

#### Scenario: Malformed scope declaration
- **WHEN** the latest intake-ready Spec scope line is empty or does not identify an own change
- **THEN** the check fails with an error instead of accepting the task as legacy

#### Scenario: A renamed requirement is also modified in a delta
- **WHEN** an own change renames an existing requirement and modifies it under the new name
- **THEN** authorization to rename the original requirement does not authorize modifying it
- **AND** modification authorization is checked against the original requirement name

#### Scenario: A foreign change is rewritten while archived
- **WHEN** a foreign change is archived with altered file contents
- **THEN** both archive and edit authorization are required for that change

#### Scenario: Text is inserted into an existing requirement
- **WHEN** a pull request adds a sentence, scenario, or scenario step inside a requirement already present in dev
- **THEN** matching modify authorization for that existing requirement is required
- **AND** adding a separate new requirement remains allowed without authorization

#### Scenario: An active foreign change is edited while copied to archive
- **WHEN** a foreign change is copied unchanged to archive while its active source is modified
- **THEN** both archive and edit authorization are required for that change

#### Scenario: A previous foreign archive is changed while another is created
- **WHEN** a pull request edits or removes an existing foreign archive and creates a new archive copy of the same change
- **THEN** archive authorization for the new copy does not authorize editing the previous archive

#### Scenario: Chained requirement renames retain their original authorization
- **WHEN** an own delta renames an existing requirement through intermediate names and modifies or removes it
- **THEN** the operation requires authorization for the original requirement in dev
- **AND** cyclic or ambiguous rename mappings fail the check

#### Scenario: A new requirement takes an existing scenario
- **WHEN** an inserted requirement heading moves existing scenario lines out of a requirement that remains in the specification
- **THEN** the check requires modify authorization for that existing requirement

#### Scenario: Reusing a renamed requirement's former name
- **WHEN** an own delta renames an existing requirement and another requirement takes its former name
- **THEN** each rename remains authorized against its own original requirement in dev
- **AND** modifying or removing the first requirement under its new name requires permission for that original requirement

#### Scenario: A requirement's text moves under another existing requirement
- **WHEN** a pull request deletes or moves the heading of an existing requirement so that some of its text or scenarios fall under another requirement already present in dev
- **THEN** authorization to remove or modify the requirement whose heading changed does not cover the change
- **AND** modify authorization for the requirement that absorbs the text is required

#### Scenario: Scope declaration names its own change ambiguously
- **WHEN** the latest intake-ready Spec scope line names no change or more than one change inside its own(...) part, or names a change outside it
- **THEN** the check fails with an error instead of treating any other word as the own change

#### Scenario: An existing archive shares the own change name
- **WHEN** a pull request edits an archive that already exists in dev under the same change name as the task's own change
- **THEN** the archive is treated as a foreign change and edit authorization is required

#### Scenario: A new capability differs from an existing one only by letter case or Unicode form
- **WHEN** a pull request adds a capability directory whose name equals an existing or another added capability after case folding and Unicode normalization
- **THEN** the check fails with an error instead of treating it as a new capability
- **AND** the same applies when a delta in the task's own change names a capability that differs from an existing one only by letter case or Unicode form
