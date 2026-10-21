# Design

## Cause

The missing exact row is a shell-startup race in the test, not lost pane output.
`open_pane` returns after creation and subscription, before the interactive shell
has necessarily printed its first prompt. The PTY can echo queued input before
that prompt. `wait_screen("sleep 1")` therefore proved only that the PTY echoed
the command. It did not prove the shell had started reading commands.

If the shell starts after the echoed command's newline, its first prompt is
printed immediately before the command's output. The screen then contains a row
such as `startup> printed later`, rather than the exact row `printed later`.
Neither waiting 1500 ms nor waiting for more frames can remove that prefix.
A reconnect correctly restores this screen, but `wait_row("printed later")`
never matches it. A line editor may redraw the queued command after the prompt,
which explains why the ordering is not equally visible with every shell.

## Deterministic reproduction and regression

`reconnect_output_when_input_precedes_the_shell_prompt` starts a wrapper blocked
on a FIFO before it execs the interactive shell. The test sends input, observes
its PTY echo, acknowledges Unsubscribe with zero viewers and disconnects before
releasing shell startup. The shell's startup file disables line editing so the
queued line is consumed without a redraw, and sets an explicit prompt.

With the old `echo "printed la""ter"` command, this produces
`startup> printed later` and fails the exact-row assertion. With
`printf '\nprinted la''ter\n'`, the leading newline isolates the expected output
from the prompt, and the same assertion passes. The split literal prevents the
input echo itself from satisfying the output wait. No sleeps or retries control
this ordering.

## Reconnect coverage

`output_printed_while_nobody_watched_is_there_on_reconnect` now uses the existing
`controlled_reconnect` fixture with the failpoints feature. Its shell script
acknowledges readiness before waiting on a FIFO. After an acknowledged
Unsubscribe, the fixture releases output, pauses unwatched output, client
acceptance, subscription and full-frame delivery in order, then checks that the
first full frame already contains the output. It no longer waits for a later
frame to repair a stale first snapshot. The lag/read-pause scenario also uses
this fixture and still checks recovery with a full frame.

The daemon and protocol require no changes. The startup regression also runs
without the failpoints feature; the full reconnect-stage scenario requires it.
