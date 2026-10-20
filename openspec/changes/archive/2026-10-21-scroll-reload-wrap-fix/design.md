# Design

## Existing requirement

`openspec/specs/reload/spec.md`, "Soft wraps carry over", requires the wrap at
the history/screen boundary to survive reload. The existing integration test
widens the pane to check the two physical rows become one logical line.

## Established cause and controlled reproduction

Resize and Scroll are separate requests. A resize alone may produce the joined
line on the live screen without showing `line-2` in history. Waiting for the
joined line after sending Scroll can therefore stop at the resize frame.
Force that ordering by receiving the joined line before sending Scroll; retain
the original assertion to establish the failure, then wait for viewport_top 0.

On 2026-10-20, the unmodified test passed on dev 4a0231f. With only the
resize-frame barrier inserted, the same narrow command exited 101 at
`assert!(client.screen.has_row("line-2"))` (task log
`logs/scroll-controlled-before.log`). This creates the offending frame ordering
without load, sleeps, retries or a new hook.

The fixed test asserts that the resize frame is still scrolled past history's
start and does not show `line-2`, then requests Scroll and receives a frame
whose `viewport_top` is zero before checking both the joined line and `line-2`.
No new timing hook is necessary: receiving the resize frame is the barrier.

Recheck with:
`cargo test -p tilrd --test scroll a_line_wrapped_across_the_history_border_stays_one_line_after_a_reload --features tilrd/failpoints --locked`.

## Validation and remaining blocker

On 2026-10-20 the fixed narrow regression passes; fresh-context self-review: ok.
The full `bin/check` ran; fmt, both clippy variants and gitleaks passed.
After adding the tests-only `skip_specs` metadata, `bin/check tilrd:openspec
tilrd:test tilrd:linux` passed. This rerun needed execution outside the sandbox:
the client test starts `ps` and Docker buildx writes its activity under ~/.docker.

The only remaining failure is cargo deny: RUSTSEC-2026-0382 for rustls-webpki
0.103.15 and RUSTSEC-2026-0381 for webpki-roots 0.26.11/1.0.9. `git diff origin/dev
-- Cargo.lock` is empty at dev 4a0231f; this task changes no dependencies.
Existing TILR-244 blocks completion. Recheck with `../bin/check tilrd:deny`;
remove this blocker once that task has repaired dev and the check passes.
