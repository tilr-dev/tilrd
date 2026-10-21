# Preserve source freshness across shared Cargo builds

## Why

The stand shares Cargo target directories between checkouts whose source mtimes may
predate cached artifacts. Recording source digests before the build completes can
claim that an untouched target directory already contains generated protocol code
from the new source tree after an interrupted or failed build.

## What Changes

- Record source digests only after the wrapped command succeeds, invalidating prior
  records before it starts so a partial build cannot certify either source tree.
- Cover failed builds and generated source freshness with a deterministic Cargo fixture.

## Impact

The `tt-fresh` wrapper and its tests; both the main and next-release target caches.
