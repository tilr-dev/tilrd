# Design

The supervisor tracks disconnected identities with a monotonic clock and cancels expiry on Hello. A one-hour constant governs retention; tests exercise the same deadline calculation with controlled time. Explicit finalization and expiry send internal removal commands to every pane. Ingress clears internal control fields. Removal drops counters and their pending delivery markers, preserving queued PTY bytes and latency marks. A finalized connection cannot admit further numbered input.

Normal clients stop admitting input, drain acknowledgements and then finalize their identity before closing. Abrupt loss keeps replay state. Expiry and exec intentionally forget delivery counters; replay can duplicate previously applied bytes, including a prefix already written before exec. The existing PTY suffix still reaches the shell.
