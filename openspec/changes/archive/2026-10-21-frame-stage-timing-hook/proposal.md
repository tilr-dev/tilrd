# Operation-bound frame timing for tests

## Why
The shared measurement clock advances by one tick per read. Concurrent changes to its file cannot reliably assign distinct durations to parse, build and encode operations.

## What Changes
Add a failpoints-only file setting that advances the measurement clock inside each frame stage by a configured duration. Cover parse accumulation and the previous-frame encode value through the real server protocol.

## Impact
Only test builds with both clock settings use the hook. Production builds retain their existing measurement operations and cost.
