## Why

The coverage report marks requirements citing Python regression tests as uncovered:
the citation parser only accepts Cargo-style identifiers. Some infrastructure
requirements also cite their tests only in the capability Purpose, outside the
requirement that the report evaluates.

## What Changes

- Recognize Python test file citations in each requirement and display them separately
  from Cargo tests in JSON and Markdown.
- Put accurate test citations on the infrastructure requirements themselves.
- Verify report generation, the cited infrastructure tests, and the final test suite.

## Impact

Coverage gains an additive `python_tests` field per requirement; `cargo_tests` keeps
its meaning. No wire protocol, feature flag, reload state, or update behavior changes.

## Non-goals

- Executing named tests during coverage generation or proving that citations exist.
- Changing server behavior or the meaning of the infrastructure requirements.

## Implementation

Implementation commit: `aca24ca` (`fix(coverage): recognize Python and crate-qualified test evidence`).
