# Bound Hello input probes

## Why

A stable client's Hello probes every live pane for applied input counters. A pane
that stops reading its pipe never answers, leaving handshakes for unrelated panes
pending until the server closes those clients for failing to greet.

## What Changes

Give Hello probes five seconds to answer. Kill unresponsive pane processes and wait
for their links to close before completing Hello. Preserve applied counters from
responsive panes and keep their input usable. A received Hello is governed by this
probe bound rather than the timeout for clients that never send Hello.
