## Ownership

The terminal guard checks whether its direct child has exited. If still running,
it kills the process group established by setsid and waits for the direct child.
Cleanup tolerates an already reaped child and does not panic while unwinding.

## Descriptor lifetime

Opening a PTY with openpty followed by fcntl leaves a fork/exec race between
allocation and setting close-on-exec. Allocate the master with posix_openpt and
O_CLOEXEC, grant/unlock it, then open the slave close-on-exec. Only duped standard
streams survive exec. This prevents inheritance from parallel tests as well.

## Verification

Use live fake-server clients, trigger ordinary drop and controlled panic, and
check waitpid reports ECHILD after cleanup. A fallback kills and reaps a survivor
before reporting a regression failure. A shell child checks that no extra tty
descriptors survive exec while another terminal client is alive.

No new server state, reload behavior, client reconnect behavior or backpressure.
