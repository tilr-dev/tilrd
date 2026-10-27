# Native FSEvents delay investigation

## Status and measured boundary

Research snapshot: 2026-10-27, macOS 26.6.2 (25G83). No product, test,
or specification behavior has been changed. The daemon's internal cause is
**not established**; a privileged stack sample and writer attribution are needed.

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
journal size. No journals were deleted and no services were restarted.

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

A clean-system or second-Mac reproduction has not been performed. Current
measurements represent one busy host and cannot establish clean-system incidence.

## Recommendations and next acquisition

For the failing file-watch investigation, retain the current product and test
deadlines while determining the daemon's work. Neither lower latency, NoDefer,
nor moving temporary files to RAM is a demonstrated remedy. Do not diagnose
journal corruption, Spotlight, cloud clients, builds, or Time Machine from CPU
usage alone. Stopping applications, restarting the daemon, deleting its journals,
and changing volume indexing are interventions requiring a controlled follow-up,
after a baseline stack and write trace have been saved.

For hybrid-watcher design, treat native delivery as a potentially delayed hint;
the fallback must have an independent trigger if timely discovery is required.
This finding does not select an architecture or establish its performance budget.
The previous design draft remains recoverable from its historical commits and
is excluded from this branch's final diff.

The owner can collect the missing privileged evidence from the task folder:

```sh
bash tilrd/diagnostic/collect-fsevents-delay.sh
```

This prompts the owner for sudo, compiles the diagnostic probe, and records a
10-second daemon sample (10 ms interval), a 90-second targeted fs_usage trace,
a 15-second all-process filesystem trace to identify writers, host metadata,
and the native matrix. Output names are `logs/privileged-<UTC timestamp>-*`.
The script is provided for the owner; the worker does not execute sudo. The
all-process trace can contain private paths and stays local.

The stack sample may distinguish active processing from lock contention,
client registration/history work, or journal operations. Writer attribution and
journal entry count provide context, not causality by themselves. A controlled
quiescent comparison should follow only after the hot stack/workload is known.
The short sample and writer window cover only the beginning of the acquisition;
absence of suspicious activity there does not exclude activity later.
The earlier original trace files should also be made available in this task's
`logs/` so the 15-second rhythm can be checked directly.

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
Compilation with warnings as errors and shell syntax validation pass. Product
checks and acceptance are pending; a zero probe exit only means acquisition
completed, not that event delivery was timely.
