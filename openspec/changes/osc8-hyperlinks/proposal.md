# Proposal: OSC 8 hyperlinks in frames

## Why

Hyperlinks (OSC 8) are listed as not done (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`); `tilr.proto` has no hyperlink field at 13f9522.

## What Changes

- To be designed: runs carry a hyperlink id, frames list the URIs in view, with a feature flag `hyperlinks`.

## Capabilities

### Modified Capabilities
- `screen-frames`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Nothing beyond What Changes.

## Impact

Additive proto fields; the pane snapshot must keep links across a reload (check libghostty-vt's formatter).
