## ADDED Requirements

### Requirement: Linux worktrees isolate build outputs
`scripts/linux.sh` SHALL mount a persistent cargo target volume specific to the physical worktree path and platform, named `tilr-linux-target-<path hash><platform suffix>`. The cargo registry SHALL remain shared per platform. `scripts/linux-build.sh` SHALL build and copy binaries from the same selected target.

**Verified by:** `linux_script::parallel_worktrees_keep_their_linux_build_outputs` and `scripts/test-linux.py --docker`.

#### Scenario: Overlapping builds
- **WHEN** worktree A builds its binary and waits, and worktree B (even with an identical directory basename) builds a different binary before A resumes
- **THEN** both commands succeed and A still runs its own binary

#### Scenario: Reuse and cleanup
- **WHEN** the same physical checkout is invoked again, including through a symlink
- **THEN** it uses the same target, which can be removed with `docker volume rm <volume name>` after its containers stop

#### Scenario: Volume naming and platform separation
- **WHEN** a checkout runs a command for a selected `TILR_LINUX_PLATFORM`
- **THEN** the path hash is Git's hash-object of its physical absolute checkout path followed by a newline
- **AND** the platform suffix replaces each non-lowercase-alphanumeric character of `TILR_LINUX_PLATFORM` with `-` (empty when unset)
- **AND** the same platform uses the same shared registry while different platforms use separate target and registry volumes

#### Scenario: Linux release copy
- **WHEN** `scripts/linux-build.sh` builds release binaries in a worktree
- **THEN** the copied `dist/linux-<arch>/tilrd` and `tilr` come from that worktree's target

### Requirement: Linux commands run their built image
`scripts/linux.sh` SHALL run the immutable image ID returned by its own successful build, even when another invocation rebuilds the shared image tag. A failed image build SHALL fail the invocation before running a container.

**Verified by:** `linux_script::parallel_worktrees_keep_their_linux_build_outputs`.

#### Scenario: Another worktree rebuilds the tag
- **WHEN** another worktree updates the platform's image tag between a build and its container start
- **THEN** the command still runs the image produced by its own build

#### Scenario: Failed build
- **WHEN** Docker fails to build the image
- **THEN** the script exits unsuccessfully without starting its command
