## Implementation

- [x] Prove the premature subscription completion with the existing registration pause
  hook in clipboard::a_copy_nobody_takes_stays_in_the_shared_buffer.
- [x] Wait for the initial full frame before the next copy; verify the same controlled
  case and exact copied bytes on Linux and macOS.
- [x] Validate formatting and both clippy configurations; require full-suite validation
  through the repository's finish gate.
