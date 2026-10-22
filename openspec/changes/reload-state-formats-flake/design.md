# Probe deadlines and compatibility verdicts

The production `check_executable` wraps process spawning, exit and stdout EOF
in one ten-second timeout. Warming an inode is not synchronization with a
subsequent invocation: a delayed successor still consumes that window. Tests
expecting a particular compatibility verdict consequently also test host
scheduling. Rejection scripts are new files even when the server inode is warm.

The controlled reproduction holds a successor on a FIFO, observes its start,
and advances Tokio time past ten seconds before releasing it. This distinguishes
the probe deadline from a format mismatch without sleeps or load-dependent
repetition. It establishes delayed completion as a sufficient cause of the
symptom; it does not identify which macOS service or scheduler delayed a
historical invocation.

With `failpoints`, explicitly selected compatibility tests set
`TILR_TEST_STATE_FORMATS_UNBOUNDED=1` and await the child result. The ordinary
test harness deadline still reports a stuck test. Without that feature, or
without the exact opt-in value, the production ten-second timeout applies.
The separate resume check retains its deadline in every configuration.

The reproduction also runs with the opt-in absent, proving that a held child
still produces `state-formats did not finish` under the production policy.
