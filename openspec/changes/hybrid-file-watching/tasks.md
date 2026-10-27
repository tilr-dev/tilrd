## Research

- [x] Inspect both watcher implementations, consumers and current spec contracts; evidence is in design.md with file references.
- [x] Compare six industry approaches using primary sources; verify links and mechanisms against the comparison table.
- [x] Propose separate bounded daemon/app schemes with work formulas, recovery and explicit latency limitations; inspect design.md.
- [ ] Obtain the owner's architecture and latency choice; verify a recorded owner decision before implementation planning is finalized.

## Implementation planning after selection

- [ ] Define the daemon implementation slice with deterministic protocol, stalled-source, admission and lifecycle tests; verify test names and required spec scope in that plan.
- [ ] Define the app implementation slice with recursive coverage, HMR, config and unload integration tests; verify site-serving spec scope and observable performance criteria.
- [ ] Implement selected behavior with tests and documented measured budgets in the implementation work; verify narrow tests, then required repository checks before completion.

This research does not implement or archive the change. Each implementation slice must include its tests and documentation rather than leave a trailing test-only stage.
