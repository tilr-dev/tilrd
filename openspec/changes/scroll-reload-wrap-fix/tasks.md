## 1. Reproduce and fix

- [x] 1.1 Read the existing reload requirement and create this task's change.
- [x] 1.2 Force the resize frame to arrive before Scroll and reproduce the original assertion failure.
- [x] 1.3 Wait for the requested viewport and verify the wrap and history on that frame.
- [x] 1.4 Record the established cause and before/after evidence.

## 2. Validate

- [x] 2.1 Run the narrow regression test.
- [x] 2.2 Fresh-context self-review and full bin/check (deny blocked by TILR-244; all other checks green, see design.md).
- [x] 2.3 Prepare the completed change for archive and Review submission via bin/finish.
