## Implementation

- [x] Specify ordinary update behavior with a legacy marker and removal of the obsolete refusal requirement.
- [x] Remove the marker guard and replace the refusal test with `update::update_installs_the_latest_release_with_a_legacy_app_marker`; update README and USE_CASES.
- [x] Wire verification through the regression test, `bin/check fmt clippy`, and the full checks and OpenSpec validation in `bin/finish`.
