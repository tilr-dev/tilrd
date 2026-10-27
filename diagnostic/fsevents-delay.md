# Native FSEvents delay investigation

## Status and measured boundary

Research snapshot: 2026-10-27, macOS 26.6.2 (25G83). No product, test,
or specification behavior has been changed. The delay is narrowed to a degraded,
long-running system daemon: the original process was CPU-bound in its notification
thread, and restarting it restored native delivery and the existing file tests.
The exact internal algorithm, allocation growth mechanism, and triggering client
are **not established**. The owner closed the request for another sudo run after
confirming recovery four hours later.

Six simultaneous, successfully started native FSEvent streams received unique
file creations without Rust, notify, or application debouncing. Each watched both
roots. All streams requested FileEvents and SinceNow, on a serviced CFRunLoop.
The only varying stream options were latency and NoDefer. Three files per root
were created, written, fsynced, and closed at approximately 2, 7, and 12 seconds.
The nominal run-loop window is 90 seconds after stream setup; setup and teardown
add wall time. This is a diagnostic acquisition window, not a changed integration-
test deadline. Delay is measured from immediately before open to native callback,
including write/fsync/close overhead, rather than strictly from write completion.
Markers are retained as evidence.

Local evidence is in the task folder's `logs/` (not committed):

| Acquisition | Roots | Observed marker delivery |
| --- | --- | --- |
| `native-matrix-20261027.log`, 09:11:19 UTC | `/private/tmp` and task-folder `logs`, both APFS Data | latency 0 with/without NoDefer and 0.001/0.2 with NoDefer: all six markers arrived together at t=54.764, 41.822–51.824 s from creation attempts. Latency 0.001/0.2 without NoDefer: no marker callbacks within the 90 s window. |
| `native-matrix-ram-20261027.log`, 09:13:47 UTC | `/private/tmp` on APFS Data and task directory on separate RAM volume | All streams received all six markers. Latency 0 with/without NoDefer and 0.001/0.2 with NoDefer: 20.827–30.263 s. Latency 0.001/0.2 without NoDefer: 22.249–32.251 s. Corresponding markers on the two volumes arrived within a millisecond of each other. |

These are two different path/volume experiments, not repeated trials until a test
passes. No delivered marker carried KernelDropped/UserDropped/MustScanSubDirs.
Absence of a loss flag cannot exclude a different internal daemon problem.
The two roots share each stream: this rules out a RAM-volume workaround in this
configuration, but does not isolate per-volume processing inside the daemon.

`host-state-20261027.log` at 09:12:01 UTC records fseventsd PID 109 at 97.6% CPU,
RSS 4,958,704 KiB (about 4.73 GiB), process age almost 60 days. It records Time
Machine Running=0, Spotlight indexing enabled, and many active applications.
CPU activity of an application is not proof it produces filesystem traffic.
APFS Data has about 1.6 TiB free (56% used); `/private/tmp` and the task folder
are the same device. The RAM comparison uses a distinct 512 MiB volume.
The protected Data `.fseventsd` directory exists; its contents and integrity have
not been inspected. Directory metadata alone establishes neither corruption nor
journal size. During these baseline acquisitions no journals were deleted and
no services were restarted; the owner subsequently restarted the daemon below.

`fseventsd-unified-20261027.log` contains group lookups and missing bundle-ID
messages for command-line clients, including this probe. It does not identify a
queue, blocking stack, or journal fault. The old inference that one worker's short
15-second bursts exclude daemon CPU overload is unsupported: it concerns one
thread, while this acquisition sees a busy process. A busy process still does
not identify the work causing delay.

The earlier privileged trace analysis belongs to the preceding investigation;
its summary is available in the task notes, but the original trace files are not
present in this task folder. Its reported 15-second rhythm has not been
independently reanalysed here. No universal 15-second FSEvents mode is established.

## Original daemon sample and restart comparison

The owner subsequently ran the collector. Existing local files with prefix
`privileged-20261027T131507Z-` capture the original PID 109, not its replacement:

- `sample-command.txt:1–3` confirms sample completion. `sample.txt:2,12–19`
  identifies PID 109, launched August 28, sampled October 27 at 14:15:12 +0100,
  physical footprint 8.3 GB. Footprint and RSS are different measurements.
- `host.txt:6,24` records about 60 days of process age, 100.1% then 99.1% CPU,
  RSS 5,509,376 then 5,257,872 KiB. It reports journal directory metadata/count,
  not journal contents or an integrity check; corruption is unproven.
- `sample.txt:24–62,254–264`: all 912 samples of the notification thread are
  in daemon userspace code; 789 (~86.5%) end at image offset `0x12ab4`.
  Small branches include mutex lock/unlock and `strncmp`. This is CPU activity
  in notification processing, without a dominant kernel wait on that thread.
  Daemon version 1413.160.2, ARM64E, UUID
  `519A5391-0608-346B-A333-D34CC76E7EC4`; internal symbols are `???`.
  Offsets cannot identify the exact algorithm or prove an infinite loop/leak.
- `sample.txt:67–89`: the volume thread waits in Mach messaging, the device
  reader in `read`, and the disklogger in a condition wait. At this sample,
  notification processing is the busy stage; active journal writing or a busy
  device reader is not what the sample shows. This short snapshot cannot
  exclude journal/history work or earlier write pressure as the trigger.
- `native.txt:1–45`: six streams start successfully. Streams 0, 1, 3 and 5
  deliver the markers near t=84.15 s, 71.465–81.467 s after creation attempts,
  across both paths. Streams 2 and 4 have no callbacks before END t=94.372.
- `fs-usage.txt:1` contains only `ktrace_start: Resource busy`: this targeted
  trace failed and provides no path timing evidence. The original collector
  attempted two concurrent fs_usage captures; the all-process capture won.
  The collector now serializes its captures to avoid that resource conflict.
- `writers.txt` spans 14:15:12.700818–14:15:28.451231 (283,383 records), dominated
  by metadata reads. It includes 1,490 `write`, 1,255 `pwrite`, 82 `writev`,
  17 `rename` and 179 `unlink` calls. These include failed calls, descriptors,
  pipes and probe traffic: counts are not filesystem-event counts. This window
  does not establish a responsible application or explain the daemon's state.

Owner observations are preserved separately in local
`logs/dan-restart-observations-20261027.md`, copied from the task snapshot
(comments #12329 and #12403); these are testimony, not raw worker-acquired traces:

| Time on 2026-10-27 | Daemon state and result |
| --- | --- |
| Around 14:29 local, before restart | PID 109, 100% CPU, RSS 5.2 GB, age 60 days; native sequence probe did not receive create. |
| About 1.5 minutes after restart | PID 51242, 0.8% CPU, RSS 13 MB; create/rename/remove/atomic-save arrive in both native streams. Existing file integration suite: 14/14 in 5.8 s at load average 26. |
| 18:22 local, about 3 h 55 min later | Same PID 51242, 0.1% CPU, RSS 9.5 MB; file integration suite on main passes 14/14 three times in 5.5–5.6 s at load average 11. |

This intervention strongly supports degraded daemon state rather than Rust,
notify, or the machine's load alone as the cause of the observed long delays.
It is not a controlled replay of identical workload: a restart also clears
queues and client state, so accumulated backlog, excessive client matching,
pathological notification processing and allocation retention remain possible.
A 60-day process age is an observed condition, not a proven failure threshold;
large RSS alone does not prove a leak. No pre-restart heap profile, symbolicated
hot function or controlled client isolation exists. The triggering history was
not preserved, so further attribution cannot be recovered from the healthy
replacement process. This is the evidential limit of the diagnosis.

## Upstream evidence and applicability

- [Apple's API guide](https://developer.apple.com/library/archive/documentation/Darwin/Conceptual/FSEvents_ProgGuide/UsingtheFSEventsFramework/UsingtheFSEventsFramework.html)
  describes stream scheduling, coalescing, and recovery after dropped events.
  Native latency is a batching parameter, not proof of end-to-end delivery time.
- [Apple's NoDefer reference](https://developer.apple.com/documentation/coreservices/kfseventstreamcreateflagnodefer)
  concerns the first event following a quiet period. The local NoDefer streams
  still experienced tens of seconds of delay; toggling it is not a sufficient fix.
- [Watchman's synchronization documentation](https://facebook.github.io/watchman/docs/cookies)
  reports late FSEvents during a large checkout on a busy host, even after a
  cookie notification and FlushSync return. Delayed delivery is therefore not a
  phenomenon unique to this host. This does not prove the same internal cause,
  exact durations, or prevalence on clean systems.
- [Watchman's configuration](https://facebook.github.io/watchman/docs/config)
  discusses heavy-write pressure and dropped-event mitigation. Such pressure is
  a hypothesis here, pending writer and stack evidence.
- [Parcel's native backend](https://github.com/parcel-bundler/watcher/blob/master/src/macos/FSEventsBackend.cc)
  uses FSEvents with latency 0.001. Replacing notify with that backend still uses
  the same service; the source provides no host-specific diagnosis.

- [Watchman's FSEvents troubleshooting](https://facebook.github.io/watchman/docs/troubleshooting.html#fsevents)
  separately describes a service entering a bad state and suggests overlapping
  watches or problematic stream clients as possible causes, with restarting the
  service among recovery options. Its example is a stream-start error, not the
  successful-but-late callbacks measured here: it supports those hypotheses,
  not an attribution of this incident to a particular client.

Sources were checked on 2026-10-27. A clean-system or second-Mac reproduction
has not been performed. Watchman's busy-host checkout case establishes that
late FSEvents can affect users beyond this Mac, but not that they share this
60-day/large-RSS failure. On this Mac the phenomenon is demonstrated in the
old CPU-bound daemon and absent in the owner's post-restart checks despite
high machine load. No prevalence estimate, uptime threshold or universal
15-second delivery mode follows from these observations.

## Recommendations and optional recurrence acquisition

For the failing file-watch investigation, resume diagnosis on the recovered host
with the existing product behavior and test deadlines. The owner has already
shown the file suite passes after daemon recovery, so a product fix for this
specific delay or larger test windows is not supported. Retain the native probe
and daemon-state evidence to distinguish any recurrence from an application bug.
Do not use daemon restarts as an automated test retry or routine product action.
Neither lower latency, NoDefer, nor moving temporary files to RAM is a demonstrated
remedy for the degraded state. There is no evidence to delete journals, disable
Spotlight/Time Machine, or blame a cloud client/build tool.

For hybrid-watcher design, native delivery can stall without a dropped-event
flag. A fallback that requires a native event to start shares that failure.
Assess an independent trigger for timely reconciliation, with explicit resource
and coverage limits and event-stream health observability. A backend replacement
that still uses FSEvents does not remove this daemon dependency. These are design
inputs, not an architecture decision or a measured performance budget. The prior
hybrid draft is excluded from this branch's final diff and belongs to the separate
design task.

No further sudo run is requested: the owner closed that request after confirming
recovery. If the original condition recurs and the owner elects to investigate
its trigger, preserve a baseline sample, daemon CPU/RSS/age and native timing
before intervening. Heap profiling/symbolication and controlled client/workload
isolation would be needed to distinguish the remaining hypotheses; the existing
artifacts do not establish them.

The retained owner-only collector can be run from the task folder during a
recurrence (the worker does not execute sudo):

```sh
bash tilrd/diagnostic/collect-fsevents-delay.sh
```

It prompts for sudo, compiles the probe, and captures a 10-second daemon sample
(10 ms interval), a 90-second targeted fs_usage trace with the native matrix,
host metadata, then a separate 15-second all-process filesystem trace. The later
writer window is context, not simultaneous attribution of the sampled hot stack.
Other running tracing tools can still prevent fs_usage from acquiring ktrace;
check each output and the printed status. Output names are
`logs/privileged-<UTC timestamp>-*`; paths may be private and stay local. The
short windows do not exclude earlier/later activity. The old trace originals
remain unavailable here; no reconstruction of their 15-second rhythm is claimed.

## Reproduce the unprivileged acquisition

Run from the task folder in a normal terminal outside the worker sandbox:

```sh
clang -Wall -Wextra -Werror -Wno-deprecated-declarations \
  tilrd/diagnostic/fsevents-delay.c -framework CoreServices \
  -o /private/tmp/native-delay
/private/tmp/native-delay > logs/native-matrix.log
/private/tmp/native-delay /Volumes/tilr-check/TILR-563 > logs/native-matrix-ram.log
```

The optional argument selects the second parent directory. The first is always
`/private/tmp`; the default second parent is the task-folder `logs/` directory.
A zero probe exit only means acquisition completed, not that event delivery
was timely; inspect START, WRITE, EVENT and END records explicitly.
