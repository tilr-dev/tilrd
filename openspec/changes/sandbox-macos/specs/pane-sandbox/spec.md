## ADDED Requirements

### Requirement: The macOS profile carries paths as parameters only
On macOS the server SHALL generate the Seatbelt profile from fixed rules and parameter names only, passing every path as a `-D NAME=VALUE` parameter, and SHALL keep it as `<runtime dir>/sandbox/<pane id>/profile.sb` (mode 0600) for the pane's life.

**Verified by**: unit tests `sandbox::macos::tests::paths_are_passed_as_parameters_only`, `sandbox::macos::tests::the_necessary_rules_are_always_present`, `sandbox::macos::tests::no_unix_socket_is_allowed_but_dns` (crates/tilrd/src/sandbox/macos.rs).

#### Scenario: A path with quotes
- **WHEN** a profile grants `read_write = ["~/a \"b\")"]` on macOS
- **THEN** `profile.sb` does not contain that path, and the pane can write in that directory

### Requirement: The sandbox is tested on every supported macOS
CI SHALL run every `sandbox::*` test and `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` on macOS 14, 15 and 26 in the merge queue, on release branches and in the nightly run on `main`.

**Verified by**: CI job `sandbox-macos` (matrix `macos-14`, `macos-15`, `macos-26`) in `.github/workflows/ci.yml`.

#### Scenario: Merge queue
- **WHEN** a pull request enters the merge queue
- **THEN** the `sandbox-macos` job runs on all three runners and the merge waits for them
