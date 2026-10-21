# Design

## D1. The files fixture raced startup root lookup

`files::nothing_outside_the_shared_directories_is_served` originally started the server
before creating `~/shared` (fixed in `417a7b9`, PR #5). The harness returns at `listening on`,
but `Files::new` resolves roots later, once. If it runs before the test creates the
directory, it leaves that root out; creating the directory afterward cannot change the
server's root list. This is a test setup race, not a product bug: the files requirement
explicitly says a root missing at startup is left out.

On 2026-10-19, a controlled pre-fix run restored that old fixture order and waited for
the existing `files on` log before creating the directory. The server logged `listening
on` at 21:14:13.985090Z, `files: root left out` at 21:14:13.998382Z, and `files on
roots=[]` at 21:14:13.998426Z. The fixture was then created, and the first `FsList
~/shared` failed with `~/shared: outside the directories this server shares`. With the
fixture created before startup, the same test passed. The permanent test waits for
`files on` after fixture creation so reverting to the old order fails deterministically,
without sleeping for a guessed interval. The original Basic Memory flake report did not
retain its failing assertion or logs; this controlled run proves the historical failure
mechanism, while the report alone cannot establish whether another later failure occurred.
