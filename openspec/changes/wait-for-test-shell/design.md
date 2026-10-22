## Cause

PaneCreated and Subscribe do not acknowledge shell startup. The PTY can echo an
input line while the shell is still starting. A shell without line editing then
prints its initial prompt immediately before executing that queued line, so an
exact output row instead contains a prompt prefix. Further frame waits cannot
remove that prefix.

## Readiness barrier

The harness isolates shell startup files. Set PS1 to `$ ` explicitly at pane
creation, since an intermediate shell can reset the inherited value. Observe its `$` row (frame rows omit trailing blank cells) on the subscribed screen
before returning from either shared opening
helper. Send no probe command: its echo would not prove shell readiness. Explicit
custom commands and the fixture holding startup remain outside this barrier.

Move the existing pre-prompt fixture into common and name its FIFO explicitly,
allowing an independent output-release FIFO without renaming either path.

## Regression

A shell startup file disables line editing, announces entry and waits on a FIFO
before its initial prompt. Drive the real shared opening helper concurrently with
an observer client. Opening must remain pending while startup is held, then return
after release. Its first command prints a split literal with no leading newline;
its exact output row must appear. Removing the barrier makes opening return while
the startup file is held.

Only test clients acquire a new wait. There is no new production state, reconnect
state, reload handover or bulk-lane buffering.
