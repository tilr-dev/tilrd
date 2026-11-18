# Tasks

## 1. The tests

- [ ] 1.1 In `bus::a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow`, send the provider's chunks in a task of their own and start the reader reading at the server's log event `participant too slow, closing`; every wait bounded, no sleeps or polling
- [ ] 1.2 Add `bus::a_reader_that_never_reads_sees_its_too_slow_connection_end_without_a_code`: the same reader reads only after the server's log event `bus participant gone` for its connection
- [ ] 1.3 Update the tests' USE_CASES.md entries

## 2. Checks and archive

- [ ] 2.1 Mutate the server to close a too-slow reader without the code and run the tests; restore
- [ ] 2.2 Run the tests with `--stress-count 200` on Linux (`scripts/linux.sh`) and on macOS on a loaded machine
- [ ] 2.3 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, gitleaks
- [ ] 2.4 Archive the change; run `openspec validate --all --strict`
