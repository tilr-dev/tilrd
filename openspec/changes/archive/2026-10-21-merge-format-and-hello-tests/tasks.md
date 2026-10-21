# Tasks

## 1. State formats

- [x] 1.1 Replace the per-case `check_formats` failure tests with one table-driven unit test; verified by `cargo test -p tilrd --bin tilrd reload::tests` (`reload::tests::check_formats_rejects_every_format_mismatch`, `reload::tests::an_executable_reading_the_same_formats_can_take_over`).
- [x] 1.2 Keep one stub substitution (another pane state format) in `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`, with the exact `state-formats` output check and the reload into the real executable afterwards; verified by `cargo test -p tilrd --test reload`.

## 2. Hello deadline

- [x] 2.1 Merge `auth::a_connection_that_never_says_hello_is_closed_after_10_s` and `metrics::a_connection_that_never_says_hello_is_counted_as_no_hello` into `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello` with every assertion of both, the `/metrics` scrape helpers shared from `tests/common/scrape.rs`; verified by `cargo test -p tilrd --test auth` and `cargo test -p tilrd --test metrics`.

## 3. Specs and docs

- [x] 3.1 Point the `**Verified by**` lines of the three requirements and the rows of `USE_CASES.md` at the new test names; verified by `openspec validate --all --strict`.
