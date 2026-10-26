# Tasks

## 0. Decision

- [x] 0.1 Owner's recorded decision (a): no service by default, hint `tilr service install`; the `installation` spec states it ("The installer sets up no service")

## 1. Implementation

- [x] 1.1 Installer changed (3ba0e93); verified by itests UC-001, UC-006, UC-023 (`uc_023.py::nothing_starts_tilrd_at_boot_unless_asked`, with a systemd user manager present)
