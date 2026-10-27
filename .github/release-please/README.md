# Release-line runner

`run.cjs` calls the pinned release-please 17.6.0 API, the engine bundled by
release-please-action v5.0.0. The action loads config and manifest files from
GitHub, so writing generated files in the checkout cannot supply these overrides.
This runner applies them in memory while retaining the existing strategies,
extra-files, App authentication, release PR labels and downstream release outputs.

The checkout must contain the complete history and tags of the current target
branch. If GitHub history differs from the checkout, the runner fails before
building a PR from a partial list of commits. `release/X.Y` selects
`X.Y.0`, then the highest stable tag of the same line plus one patch. RC tags and
newer release lines do not affect that version. For the first release of a line,
the highest earlier stable version supplies the shipped baseline. Annotated tags
are peeled to commits. A divergent same-line stable tag fails before any writes.

The common ancestor bounds the changelog. `git cherry` additionally removes
patch-equivalent fixes already cherry-picked onto the previous release branch;
the SHA on main can differ. With no stable tag the configured bootstrap bounds
history. Nothing writes version files or release commits into main.

Merged release PRs are published using the checked-in manifest before considering
new PRs. Version and history overrides apply only to building the next PR. A
release push returns its tag through the existing root or package-prefixed outputs;
a candidate push leaves release_created false, so tilrd's RC step still runs.

Run the deterministic local Git fixtures with:

```sh
npm ci --prefix .github/release-please --ignore-scripts --no-audit --no-fund
npm test --prefix .github/release-please
```

Tests exercise the actual release-please strategies against local Git history and
stub only GitHub I/O. CI runs them on pull requests and merge groups. When changing
the engine version, update the lockfile and run both repository test suites.
