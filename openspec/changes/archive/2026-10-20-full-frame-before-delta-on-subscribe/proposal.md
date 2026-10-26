# Full frame before delta on subscribe

## Why
A newly registered subscriber can receive a delta already in flight from the pane before the full frame requested by Subscribe (TILR-183).

## What Changes
- Hold frame delivery for each Subscribe until a full frame arrives.
- Preserve acknowledgements of the subscriber’s own input in skipped frames using the existing full-frame recovery path.
- Prove the ordering with a fake pane channel, without timing or retries.

## Impact
Supervisor frame routing and regression tests; no wire protocol changes.
