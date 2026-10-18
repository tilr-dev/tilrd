## ADDED Requirements

### Requirement: Two servers of one user keep their own host sockets
Two servers of one user running at once SHALL each have a host socket of their own, and neither MUST remove the other's.

#### Scenario: Two daemons
- **WHEN** two `tilrd` of one user run on different Unix sockets with one state directory
- **THEN** `tilr edit` works in panes of both
