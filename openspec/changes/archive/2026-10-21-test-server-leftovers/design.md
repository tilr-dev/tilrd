# Test process ownership

The integration harness used Rust destructors as its only ownership boundary.
SIGTERM and SIGKILL terminate the test executable without running those
destructors. Direct supervisors remain, client-started daemons deliberately
double-fork into their own sessions, and panes also run in separate sessions.
Killing the test process group alone therefore cannot define their lifetime.

There were two additional gaps: startup read stderr before constructing its
server guard, and client test machines stopped daemons only through a pathname
that their own tests can remove. The old two-second destructor escalation also
raced with the server's longer pane shutdown sequence.

Each test owner now listens on a private Unix socket inside its scratch
directory. A server opts into the internal `TILR_TEST_OWNER_SOCKET` hook,
connects before readiness and identifies its PID. The harness registers a
kqueue process exit event on macOS or pidfd on Linux before acknowledging
startup. An owner retains accepted connections independently of the server's
command socket and executable path. Closing them requests ordinary SIGTERM
shutdown in the supervisor, including pane and shell cleanup. Process death
closes these connections even if no destructor runs. Client daemonization
requires no special signal forwarding or inherited descriptors.

The server's ownership stream is close-on-exec. A reload reconnects in the new
image, to the same owner, before readiness; old connections are retained until
owner cleanup. The server's ownership thread starts after signal handling is
installed. Ordinary launches without the test environment variable do not
connect or start an ownership thread.

The server guard is constructed immediately after spawning, before startup
output is parsed. Client test machines close ownership before deleting their
private installation. Destructors wait for registered process exit events,
then reap direct children and remove scratch files. A bounded failure deadline
and final SIGKILL report a broken graceful shutdown; polling and sleeps do not
order successful cleanup.

The external stand runs controlled panic, startup panic, SIGTERM and SIGKILL
following a process wait timeout against both original and copied executables.
It registers exit watchers for the ready supervisor, pane and shell before
triggering interruption. A separate client regression panics with a copied
daemon's command socket removed. Each outcome uses exact owned PIDs; macOS
also intersects pgrep output with the owned supervisor. Stand cleanup retains
which exit events it consumed, so it never waits for an already consumed event.
