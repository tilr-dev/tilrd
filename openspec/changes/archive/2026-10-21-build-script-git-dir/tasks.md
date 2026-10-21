# Tasks

## 1. Build scripts

- [x] 1.1 Resolve `HEAD` and `logs/HEAD` through `git rev-parse --git-path` in both build scripts and watch only the files that exist; watch none when git cannot name them.
- [x] 1.2 Add `build_script` tests to `tilr` and `tilrd` that check every watched file exists and that the git files are watched when git names them.

## 2. Checks

- [x] 2.1 Run the repository checks and archive this change.
