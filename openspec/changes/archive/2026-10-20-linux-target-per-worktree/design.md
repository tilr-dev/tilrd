## Design

Use `tilr-linux-target-<git hash-object of physical checkout path plus newline><platform suffix>` instead of the platform-only name. The full path distinguishes worktrees even when their directory basenames match; a physical path makes symlink entry points reuse the cache. Git is already required to use this repository. Keep the existing platform suffix and shared cargo registry. A lock would serialize independent tasks; separate target directories let them run concurrently.

Capture `docker build --quiet`'s immutable image ID and pass it to `docker run`. The existing platform image tag remains available, but another worktree rebuilding that tag cannot switch the image between build and run.

`linux-build.sh` builds and copies from `/target` within a single `linux.sh` invocation, so it automatically uses the selected volume. On 2026-10-20, inspection of task `bin/check` shows no Linux-specific volume setting; no workflow change is required (recheck: `rg -n 'TILR_LINUX|linux.sh' bin/check`).

## Deterministic regression

The fixture makes two actual Git worktrees with the same basename and different source contents. A builds, reports completion and waits on a FIFO; B builds into its target while A remains running; A then resumes and checks its executable. The mock Docker models the shared output path for CI without requiring Docker. The optional real-Docker mode compiles two small Rust binaries using the real Linux image and scripts, and checks that A still prints A after B builds B. Reverting the target selection must fail the same check. No scheduling delay or repeated lucky run is involved.

There is no new runtime state, reconnect behavior, bulk data or backpressure change.

## Reproduction evidence (2026-10-20)

`git show 3f9b9f8:scripts/linux.sh > ../logs/linux-original.sh` and
`python3 scripts/test-linux.py --docker --linux-script ../logs/linux-original.sh`
fail with `AssertionError: A observed B` (log: task `logs/linux-docker-baseline.log`).
The unchanged script is copied into both temporary Git worktrees; only the old
shared volume name is redirected to a fixture-specific volume to protect other
workers. Both tiny Rust workspaces name their binaries `tilrd` and `tilr` but
print different markers. A finishes compiling before B starts; B's source mtime
is fixed in the future to force Cargo to rebuild the shared output. B succeeds,
then A's resumed command runs B's binary from the common `/target/debug/tilrd`.
This proves the shared output path can replace a running worktree's build; it
does not claim to reproduce the historical transient EACCES specifically.

With the fixed script, `python3 scripts/test-linux.py --docker` exits 0:
`overlapping builds: A retained A after B built B` and
`target reuse, platform isolation, release copy: ok`
(task `logs/linux-docker-fixed.log`). The cargo integration test also passes.
