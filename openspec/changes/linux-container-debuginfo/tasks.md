# Tasks

## 1. Spec

- [x] 1.1 `release-process` delta: the Linux dev container builds the dev profile with line-tables-only debuginfo; verify with `openspec validate linux-container-debuginfo --strict`

## 2. Script

- [x] 2.1 `scripts/linux.sh` passes `-e CARGO_PROFILE_DEV_DEBUG=line-tables-only` to `docker run`; verify with `release::the_linux_dev_container_builds_with_line_tables_only_debuginfo`

## 3. Test

- [x] 3.1 `release::the_linux_dev_container_builds_with_line_tables_only_debuginfo` in `crates/tilrd/tests/release.rs` (static check of `scripts/linux.sh`), red without 2.1; USE_CASES.md row; verify with `cargo test -p tilrd --test release the_linux_dev_container`

## 4. Container check

- [ ] 4.1 After a `scripts/linux.sh cargo build -p tilrd`, `target/debug/tilrd` in the container is close to the host's size, and `scripts/linux.sh cargo test -p tilrd --test update --locked` takes about as long as on macOS
