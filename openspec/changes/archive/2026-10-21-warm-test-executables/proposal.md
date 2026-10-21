# Warm test executables

## Why

Starting each newly copied executable triggers a fresh macOS security assessment.
Reload and update tests can reuse warmed executable inodes while keeping private
installation paths and genuine executable replacement.

## What Changes

- Warm the original server and one private copy once per integration-test binary.
- Hardlink private installations to these inodes.
- Alternate inodes when replacing an executable so rename changes the file.

## Impact

Shared test harness and reload/update integration tests only.
