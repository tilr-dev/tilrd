# Tasks

## 1. Leftover servers

- [x] 1.1 Identify the leak: test-process signals bypassed destructor-only ownership; startup parsed readiness before constructing its guard; client cleanup depended on a removable command socket. Introduce independent ownership connections and process exit observation for direct and daemonized servers.
- [x] 1.2 Add deterministic panic, startup panic, signal and controlled timeout cases for original and copied executables, including pane/shell exit and a missing daemon command socket.

## 2. Checks

- [x] 2.1 Run the repository checks and archive this change.
