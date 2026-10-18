# Design (draft; deferred)

## D1. Quarantine experiment (first, may end the macOS approach)
A minimal signed binary with `com.apple.security.app-sandbox` (and an embedded Info.plist if a bare CLI binary needs one) starts a shell. Inside it: write and run `./build.sh`; `cargo build && ./target/debug/x`; check `xattr -l` for `com.apple.quarantine` on both. Pass: both run. Also record what fails (`ps`/`pgrep` EPERM is known). Result goes into this design and Basic Memory.

## D2. macOS
The supervisor (not sandboxed) spawns the sandboxed pane binary instead of `tilrd pane`; it is the pane process (pty, screen model) or only the shell's parent (to decide after D1). Entitlements are static per signed binary, so per-pane paths come from file exceptions or security-scoped bookmarks: open question.

## D3. Linux
In the pane's `pre_exec` before the shell: build a Landlock ruleset from the request (read everywhere, write only the allowed paths; with ABI ≥ 4 (kernel 6.7) TCP bind/connect limited), `landlock_restrict_self`. Children inherit it. If the kernel's ABI is too old for the request, `CreatePane` fails with a message; `sandbox` is not listed when Landlock is absent.

## Reload and reconnect
The request is stored in the pane state; the restriction lives in the kernel on the shell's process tree and is not affected by the pane process's exec. Clients reconnect as on any reload.

## Open questions
- Per-pane write paths with static App Sandbox entitlements.
- Loopback alias IPs per pane (Basic Memory note) under App Sandbox and Landlock.
