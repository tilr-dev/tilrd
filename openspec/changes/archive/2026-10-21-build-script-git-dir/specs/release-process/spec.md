## ADDED Requirements

### Requirement: Builds are rebuilt only when the commit changes
The build scripts of `tilr` and `tilrd` SHALL watch the real git directory's `HEAD`
and `logs/HEAD`, resolved with `git rev-parse --git-path` so that clones and git
worktrees alike name files that exist, and SHALL watch no file when git cannot name
them or they do not exist.

**Verified by**: `build_script::every_watched_file_exists_and_the_git_head_is_watched` (in `tilr` and in `tilrd`).

#### Scenario: Repeated build in a git worktree
- **WHEN** `cargo build` runs twice without changes in a git worktree
- **THEN** the second build compiles nothing

#### Scenario: Checkout in a clone
- **WHEN** HEAD moves to another commit
- **THEN** the next build embeds the new commit in the version

#### Scenario: Build without git
- **WHEN** git is unavailable
- **THEN** no file is watched and the version is `<package>-dev`
