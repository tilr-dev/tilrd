## 1. Shared shell readiness

- [x] Add the delayed-startup regression and its USE_CASES entry; verify `reconnect::opening_a_fresh_pane_waits_for_the_first_shell_prompt` fails without the readiness barrier.
- [x] Verify ordered prompts from different panes with `reconnect::shell_readiness_ignores_prompts_from_other_panes`.
- [x] Await the isolated first prompt in shared opening helpers; verify the regression's first exact output row.
- [x] Share the pre-prompt fixture and name independent FIFOs; verify the existing reconnect startup scenario.
- [x] Document the harness contract and validate with the repository checks on macOS and Linux.
