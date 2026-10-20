## Implementation and verification

- [x] Specify target isolation and image selection (strict OpenSpec validation).
- [x] Implement per-worktree targets and document cache cleanup (`linux_script::parallel_worktrees_keep_their_linux_build_outputs`).
- [x] Reproduce old-volume corruption and verify overlapping builds with a FIFO barrier (`scripts/test-linux.py --docker`, regression fails before fix).
- [ ] Run required repository checks with `bin/check` and finish with `bin/finish`.
