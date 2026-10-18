## ADDED Requirements

### Requirement: An existing state directory is made private
At start the server SHALL set an existing state directory of its user to mode 0700.

#### Scenario: Loose state directory
- **WHEN** `~/.local/state/tilr` exists with mode 0755
- **THEN** after `tilrd` starts it is 0700
