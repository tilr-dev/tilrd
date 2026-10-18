"""UC-035: the Unix socket of a pane's `$TILR_SOCK` (host events: `tilr edit`, `tilr
open`) is the user's alone: another uid cannot send `tilr edit` to the user's client,
not even where the permissions were opened by hand (the server checks the peer's uid, as
on its main socket, UC-034). The pane's SSH agent socket (`$SSH_AUTH_SOCK`) likewise.

The user's tilrd is started by `tilr`; a driver with "host-events" watches the pane over
TCP with the user's token, so a request that got through would reach it.
"""

import os
import shutil
import stat
import time
from pathlib import Path

from tt import CLIENT_EXE, T, case

NOBODY = 65534


@case(spec=[
    "access-control/the-host-events-and-agent-sockets-check-the-peer-s-uid",
    "clipboard-host-events/the-host-socket-is-private-and-goes-with-the-server",
    "clipboard-host-events/the-host-socket-serves-the-user-s-own-programs-only",
    "ssh-agent-forwarding/only-the-user-s-own-programs-may-use-the-socket",
])
def another_user_cannot_send_tilr_edit_to_the_pane(t: T):
    """UC-035: uid 65534 cannot reach $TILR_SOCK (permissions, then the server's uid check); the user can."""
    user = t.user()
    user.prompt()
    t.eq(user.run("tilr", "new", "-d").code, 0, "tilr new -d")
    token = (user.home / ".local/state/tilr/token").read_text().strip()
    d = t.driver()
    d.connect("ws://127.0.0.1:7890", token=token, features=["host-events", "agent"])
    pane = d.shell_pane()
    env = {name: d.value(pane, f"${name}") for name in ("TILR_SOCK", "TILR_PANE", "TILR_SERVER", "SSH_AUTH_SOCK")}
    sock = Path(env["TILR_SOCK"])
    t.eq(stat.S_IMODE(sock.stat().st_mode), 0o600, f"{sock} is 0600")
    t.eq(stat.S_IMODE(sock.parent.stat().st_mode), 0o700, f"{sock.parent} is 0700")

    def as_nobody():
        since = d.mark()
        vars = " ".join(f"{k}={v}" for k, v in env.items())
        r = t.sh(f"cd /tmp && exec setpriv --reuid={NOBODY} --regid={NOBODY} --clear-groups "
                 f"env -i PATH=/usr/bin:/bin HOME=/nonexistent {vars} {shutil.which(CLIENT_EXE)} edit /etc/hostname")
        t.ok(r.code != 0, f"another user's `tilr edit` fails: {r.text}")
        time.sleep(2)
        t.eq([e for e in d.events[since:] if e.get("ev") == "host_event"], [], "no request reached the client")
        return r

    r = as_nobody()
    t.has(r.err.lower(), "permission denied")

    # Opened by hand: the kernel lets it connect, the server turns it away.
    os.chmod(sock.parent, 0o755)
    os.chmod(sock, 0o666)
    as_nobody()
    log = (Path("/tmp") / f"tilr-{user.uid}" / "tilrd.log").read_text()
    t.has(log, "host socket: refused a connection", f"uid {NOBODY}")

    # The agent socket of the pane, opened by hand too.
    agent = Path(env["SSH_AUTH_SOCK"])
    os.chmod(agent.parent.parent, 0o755)  # agent/
    os.chmod(agent.parent, 0o755)  # agent/<pid>/
    os.chmod(agent, 0o666)
    r = t.sh(f"exec setpriv --reuid={NOBODY} --regid={NOBODY} --clear-groups python3 -c "
             f"'import socket; s = socket.socket(socket.AF_UNIX); s.connect(\"{agent}\"); s.recv(1)'")
    t.eventually(lambda: "agent socket: refused a connection" in
                 (Path("/tmp") / f"tilr-{user.uid}" / "tilrd.log").read_text(),
                 "the server refuses another uid on the agent socket", timeout=5)
    t.lacks("\n".join(str(e) for e in d.events), "stream_open")

    # The user, in the pane: it goes through.
    since = d.mark()
    t.eq(d.run(pane, "tilr edit notes.txt"), 0, "the user's own `tilr edit`")
    got = d.wait_event("host_event", since=since, timeout=10)
    t.has(got["path"], "notes.txt")
