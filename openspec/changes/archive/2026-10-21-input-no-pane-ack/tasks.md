## Implementation

- [x] Add a shared missing-pane rejection acknowledgement for all numbered pane-input variants; verify with `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`, including the failure without the fix.
- [x] Document terminal acknowledgement semantics in the protocol comment and README, and the regression in USE_CASES.md; verify against the ADDED protocol requirement.
- [x] Complete formatting, lint and repository checks through the task check/finish commands, including Linux and OpenSpec validation.
