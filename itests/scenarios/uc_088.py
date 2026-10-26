"""UC-088: `cloudflared` does not outlive the server. It runs in a process group of its own
(a signal to the server's group does not reach it) and, on Linux, with a parent-death
signal: a server killed with SIGKILL, which runs no cleanup, still takes it along, and it
gets SIGTERM, so it signs off at Cloudflare.

Cloud is played by the case (tt/cloud.py) as in UC-07A; `cloudflared` is its stand-in
script, which notes its pid and the signal that ended it.
"""

import os
import signal
from pathlib import Path

from tt import T, case
from tt.cloud import fake_cloudflared

TUNNEL = {
    "id": "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4",
    "account_tag": "0123456789abcdef0123456789abcdef",
    "secret": "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0cw==",
    "hostname": "abc234.tilr.test",
}


def flare_pid(flare_dir: Path) -> int:
    try:
        return int((flare_dir / "pid").read_text())
    except (OSError, ValueError):
        return 0


@case(fixture="server_state", spec=[
    "cloud-tunnels/cloudflared-does-not-outlive-the-server#sigkill",
])
def cloudflared_is_gone_after_the_server_is_sigkilled(t: T):
    """UC-088: SIGKILL to a server with its tunnel up: its cloudflared gets SIGTERM and is gone within 5 s."""
    t.need("procfs")
    cloud = t.cloud()
    state = t.scratch("cloud-state")
    (state / "token").write_bytes((t.root / "state" / "token").read_bytes())
    os.chmod(state / "token", 0o600)
    cloud.link(state)
    flare_dir = t.scratch("cloudflared")
    server = t.server("--cloudflared", fake_cloudflared(flare_dir), state=state)
    control = cloud.accept()
    control.greet()
    control.tunnel_is("down")
    control.send({"type": "tunnel_open", "tunnel": TUNNEL})
    control.tunnel_is("up")
    pid = flare_pid(flare_dir)
    flare = t.proc(pid)
    t.ok(flare and flare.alive and flare.ppid == server.pid, f"cloudflared runs as the server's child (pid {pid})")
    t.ok(flare.pgid == pid and flare.pgid != t.proc(server.pid).pgid,
         f"in a process group of its own ({flare.pgid}, the server's {t.proc(server.pid).pgid})")

    server.signal(signal.SIGKILL)
    t.eventually(lambda: not ((p := t.proc(pid)) and p.alive), "cloudflared is gone", timeout=5)
    ended = (flare_dir / "ended").read_text().strip() if (flare_dir / "ended").exists() else "killed"
    t.eq(ended, "TERM", "cloudflared was told to stop (SIGTERM), so it signs off at Cloudflare")
