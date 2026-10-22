# Launch-policy queue latency and probe deadlines

## Measured cause

On macOS on 2026-10-22, six concurrent parallel reload test binaries reproduced
real `state-formats did not finish` failures in both the format-mismatch and
executable-replacement scenarios. Diagnostic instrumentation measured parent
spawn, first stdout, exit, and the tilrd child's first entry into `main`.
A simultaneous `log stream --level debug` captured syspolicyd's launch assessment.

| Successor | Spawn return | Scan queued | Scan started | Assessment released | Probe deadline |
| --- | --- | --- | --- | --- | --- |
| Replacement tilrd | 13:38:00.598908 | 13:38:00.600 | 13:38:09.944 | 13:38:10.667 | 13:38:10.598082 |
| Format-mismatch script | 13:38:01.205050 | 13:38:01.206 | 13:38:11.379 | 13:38:12.186 | 13:38:11.205819 |

Times are local Europe/Zurich. The replacement's spawn took 1.85 ms and the
script's took 0.64 ms. The replacement never entered `main` before the timeout.
The respective Gatekeeper scan queue waits were 9.344 and 10.173 seconds;
the scans themselves took approximately 0.72 and 0.81 seconds. Both assessments
ran through the same serialized scan worker. syspolicyd's completion records
identify the successor paths and PIDs, establishing the link to the failed probes.
The dominant delay was therefore the OS launch-policy queue before user code,
rather than a slow format calculation or a parent runtime failing to poll stdout.

A separate four-binary experiment sampled a delayed shell successor for one
second: every sample remained at `_dyld_start + 0`, with a 96 KiB footprint.
With one, two, and four parallel reload binaries, the longest completed probes
were approximately 1.72, 4.41, and 8.25 seconds. These measurements describe the
observed workload, not a guaranteed latency bound.

## Limits of warming

The tests already warm shared executable inodes. Nevertheless, the two failed
successors received fresh launch assessments. The logs do not establish why a
previous warm invocation did not satisfy those assessments. Scripts are also
newly written successor files, independent of the warmed daemon inode pool.

A separate copy-and-hardlink experiment ran `--version` repeatedly: the first
launch took 635 ms, while subsequent launches, including a new hardlink and a
launch after unlinking it, took 7–9 ms. Thus creating a hardlink or changing its
link count alone is not an established explanation for the fresh assessments.
No cache invalidation mechanism is assumed here.

## Required separation

The production probe currently includes OS launch admission, process execution,
stdout EOF, and exit in one ten-second deadline. Real reloads can encounter the
measured queue too; the measurement does not justify changing their policy
implicitly through a test fix. The production and resume-check limits remain
ten seconds.

The earlier unbounded completion opt-in removes the probe deadline throughout
execution, while the compatibility harness still imposes its own log/client
windows. It therefore does not isolate the measured startup phase, and its FIFO
regression demonstrates only the removal of a deadline.

A test-only synchronization design should identify the successor's first
user-space readiness event, allow the OS admission phase to complete before
starting compatibility assertions, and retain the existing execution deadline
after that event. A controlled startup hold must reproduce the measured
pre-execution queue separately from a child that hangs after readiness. The
latter must still time out. Any production policy change requires a separate
explicit decision.
