## ADDED Requirements

### Requirement: A finished write is on disk
The server SHALL sync a written file's data and the rename into its directory to disk before it answers the write as done.

#### Scenario: Write then power loss
- **WHEN** a client gets the success answer of an `FsWrite`
- **THEN** the file's new content and name have been synced to disk
