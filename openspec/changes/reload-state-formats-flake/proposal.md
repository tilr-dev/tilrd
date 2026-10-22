# Isolate reload compatibility tests from probe scheduling

## Why

Warming executable inodes removes a cold-copy assessment from reload tests,
but does not guarantee that a successor finishes within the production probe
deadline. A compatibility assertion can therefore receive a timeout instead
of the format or command-line verdict it is testing.

## What Changes

- Reproduce a held successor across the probe deadline with virtual time.
- Let selected compatibility tests await completion through a `failpoints`-only
  environment switch, retaining their outer test deadlines.
- Keep the production ten-second limit and the successor resume-check limit.

## Capabilities

- Add a test-stand requirement for isolating compatibility verdicts from the
  production probe deadline.
