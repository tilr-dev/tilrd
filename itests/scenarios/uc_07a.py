"""UC-07A: tilr Cloud over the control channel: `tunnel_open` starts `cloudflared`, aimed at
this server's port and nowhere else, and `tunnel_close` stops it; nothing else Cloud
sends is acted on: Cloud does not touch panes (the trust model of
crates/tilrd/src/cloud.rs: no message from Cloud reaches a pane).

The case plays Cloud (tt/cloud.py: the WebSocket the linked server keeps open, JSON
messages) and links the server to it with the file `tilrd cloud login` leaves
(state/cloud.json). `cloudflared` is a stand-in script (--cloudflared) that notes its
arguments, its config, its pid and how it ended, says it registered and runs until
stopped.
"""

import json
import os
import stat
import time
from pathlib import Path

from tt import T, case
from tt.cloud import fake_cloudflared

TUNNEL = {
    "id": "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4",
    "account_tag": "8e0530af975a33f2f739127f75084531",
    "secret": "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0cw==",
    "hostname": "abc234.tilr.test",
}


def linked(t: T, *args):
    cloud = t.cloud()
    state = t.scratch("cloud-state")
    for name in ("token",):
        src = t.root / "state" / name
        (state / name).write_bytes(src.read_bytes())
        os.chmod(state / name, 0o600)
    cloud.link(state)
    flare_dir = t.scratch("cloudflared")
    flare = fake_cloudflared(flare_dir)
    server = t.server("--cloudflared", flare, *args, state=state)
    control = cloud.accept()
    t.eq(control.path, "/v1/machine/connect", "the server connects to Cloud's control address")
    t.eq(control.authorization, "Bearer tilr_m_test", "with the machine token")
    hello = control.greet()
    return cloud, control, hello, server, flare_dir


def flare_pid(flare_dir: Path) -> int:
    try:
        return int((flare_dir / "pid").read_text())
    except (OSError, ValueError):
        return 0


@case(fixture="server_state")
def tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it(t: T):
    """UC-07A: tunnel_open -> cloudflared runs with a config pointing at this server's port; tunnel_close -> it is stopped."""
    cloud, control, hello, server, flare_dir = linked(t)
    t.eq((hello["protocol"], hello["auth"]), (1, True), "the server's hello: protocol 1, with authentication")
    t.eq(control.tunnel_is("down")["state"], "down", "no tunnel at first")
    t.ok(not flare_pid(flare_dir), "and no cloudflared")

    opened = time.monotonic()
    control.send({"type": "tunnel_open", "tunnel": TUNNEL})
    control.tunnel_is("up")
    t.metric("tunnel_up_ms", round((time.monotonic() - opened) * 1000))
    pid = flare_pid(flare_dir)
    t.ok(pid and t.proc(pid) and t.proc(pid).alive, f"cloudflared runs (pid {pid})")
    t.ok(t.proc(pid).ppid == server.pid, "as the server's child")
    args = (flare_dir / "args").read_text().split()
    t.eq((args[0], args[1], args[3]), ("tunnel", "--config", "run"), "cloudflared tunnel --config FILE run")
    config = (flare_dir / "config").read_text()
    t.has(config, f"tunnel: {TUNNEL['id']}\n",
          f"ingress:\n  - hostname: {TUNNEL['hostname']}\n    service: http://127.0.0.1:{server.port}\n"
          "  - service: http_status:404\n")
    credentials = Path(args[2]).parent / "credentials.json"
    t.eq(json.loads(credentials.read_text())["TunnelSecret"], TUNNEL["secret"], "the secret is in the credentials")
    t.eq(oct(stat.S_IMODE(credentials.stat().st_mode)), "0o600", "readable by the server's user only")

    closed = time.monotonic()
    control.send({"type": "tunnel_close"})
    control.tunnel_is("down")
    t.eventually(lambda: not (t.proc(pid) and t.proc(pid).alive), "cloudflared is gone", timeout=5)
    t.metric("tunnel_close_ms", round((time.monotonic() - closed) * 1000))
    t.eq((flare_dir / "ended").read_text().strip() if (flare_dir / "ended").exists() else "killed", "TERM",
         "cloudflared was asked to stop (SIGTERM), not killed")
    t.ok(not credentials.parent.exists(), "the tunnel's config and credentials are removed")

    control.send({"type": "tunnel_open", "tunnel": TUNNEL})
    control.tunnel_is("up")
    t.ok(flare_pid(flare_dir) not in (0, pid), "a tunnel opened again runs a new cloudflared")


@case(fixture="server_state")
def nothing_but_tunnel_commands_from_cloud_is_acted_on(t: T):
    """UC-07A: Cloud sends pane, input, exec, file and clipboard commands: none has any effect; the server carries on."""
    cloud, control, hello, server, flare_dir = linked(t)
    control.tunnel_is("down")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane()
    marker = t.scratch("cloud") / "pwned"
    before_panes = d.list()
    before_screen = d.text(pane)
    hostile = [
        {"type": "create_pane", "command": ["touch", str(marker)]},
        {"type": "input", "pane_id": pane, "data": f"touch {marker}\r", "text": f"touch {marker}\r"},
        {"type": "key", "pane_id": pane, "text": "x"},
        {"type": "paste", "pane_id": pane, "text": f"touch {marker}\r"},
        {"type": "close_pane", "pane_id": pane},
        {"type": "exec", "argv": ["touch", str(marker)]},
        {"type": "run", "command": f"touch {marker}"},
        {"type": "fs_write", "path": str(marker), "data": "x"},
        {"type": "clipboard_put", "mime": "text/plain", "data": "from cloud"},
        {"type": "update", "cdn": "http://127.0.0.1:1"},
        {"type": "token"},
        {"type": "subscribe", "pane_id": pane},
    ]
    for message in hostile:
        control.send(message)
    time.sleep(2)
    t.ok(not marker.exists(), "nothing Cloud sent ran anything")
    t.eq([p["pane"] for p in d.list()], [p["pane"] for p in before_panes], "no pane made or closed")
    t.eq(d.text(pane), before_screen, "nothing typed into the pane")
    t.ok(server.alive(), "the server runs on")
    t.eq(control.closed, None, "the control connection stays")
    t.eq([m for m in control.seen if isinstance(m, dict) and m.get("type") not in ("hello", "tunnel_status")], [],
         "Cloud was told nothing about the server's panes or files")

    def a_tunnel_cannot_be_turned_elsewhere():
        bad = dict(TUNNEL, hostname="a.dev\n  - service: ssh://localhost:22")
        control.send({"type": "tunnel_open", "tunnel": bad})
        t.has(control.tunnel_is("failed")["detail"], "hostname")
        t.ok(not flare_pid(flare_dir), "no cloudflared for a tunnel that would rewrite the config")
        control.send({"type": "tunnel_open", "tunnel": dict(TUNNEL, service="http://127.0.0.1:22"),
                      "service": "http://127.0.0.1:22", "local": "127.0.0.1:22"})
        control.tunnel_is("up")
        config = (flare_dir / "config").read_text()
        t.has(config, f"service: http://127.0.0.1:{server.port}\n")
        t.lacks(config, ":22\n")
    t.check("Cloud cannot point the tunnel anywhere but this server's port", a_tunnel_cannot_be_turned_elsewhere)


@case(fixture="server_state")
def a_server_without_authentication_gets_no_tunnel(t: T):
    """UC-07A: a server started with --no-auth refuses tunnel_open (failed, no cloudflared)."""
    cloud, control, hello, server, flare_dir = linked(t, "--no-auth")
    t.eq(hello["auth"], False, "the hello says there is no authentication")
    control.tunnel_is("down")
    control.send({"type": "tunnel_open", "tunnel": TUNNEL})
    t.has(control.tunnel_is("failed")["detail"], "without authentication")
    time.sleep(1)
    t.ok(not flare_pid(flare_dir), "no cloudflared was started")
