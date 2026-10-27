## ADDED Requirements

### Requirement: Applied resize dimensions precede delayed input acknowledgement
When a numbered Resize is applied while earlier numbered input is still waiting to be fully written to the PTY, the server SHALL update the pane's listed dimensions and push a PaneList without waiting for the cumulative input acknowledgement. It SHALL keep that acknowledgement behind the earlier PTY input.

**Verified by**: `input_delivery::a_numbered_resize_updates_the_pane_list_while_pty_input_waits`.

#### Scenario: Earlier PTY input is blocked
- **GIVEN** a pane program waiting on an external gate without reading its PTY
- **AND** numbered raw input larger than the PTY's capacity is pending
- **WHEN** a later numbered Resize is applied to 120 columns and 40 rows
- **THEN** a pushed PaneList and ListPanes report 120 columns and 40 rows before the gate opens
- **AND** reconnecting reports that neither the raw input nor the Resize has been cumulatively acknowledged
- **AND** after the program reads the pending bytes, the Resize is cumulatively acknowledged
