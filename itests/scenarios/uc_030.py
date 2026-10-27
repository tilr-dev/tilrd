"""UC-030: by default tilrd listens on 127.0.0.1 only: the TCP port (7890) is there for the
Tilr app and tunnels on this machine, and closed to another container.

A tilrd started by `tilr` (no flags) and one started bare as a service would start it
(`tilrd`) both bind 127.0.0.1:7890 and nothing else; a client with the token reaches it
on loopback; from the host `peer` the case's address refuses the port.
"""

import re

from tt import T, case


def listeners(t: T) -> list[str]:
    return [line.split()[3] for line in t.run("ss", "-Hltn", "sport = :7890", quiet=True).out.splitlines()]


def closed_from_peer(t: T):
    peer = t.host("peer")
    r = peer.sh(f"python3 -c 'import socket; socket.create_connection((\"{peer.peer_ip}\", 7890), timeout=5)'")
    t.ok(r.code != 0, f"port 7890 of {peer.peer_ip} is closed to another container: {r.text}")
    t.ok(re.search("refused|timed out", r.err, re.I), "refused (or dropped)")


@case(hosts=["peer"], spec=["daemon-lifecycle/tcp-port-defaults-to-loopback-7890"])
def a_tilrd_tilr_starts_listens_on_loopback_only(t: T):
    """UC-030: `tilr` starts tilrd: 127.0.0.1:7890 only; loopback works with the token, another container is refused."""
    user = t.user()
    user.prompt()
    t.eq(user.run("tilr", "new", "-d").code, 0, "tilr new -d")
    t.eq(listeners(t), ["127.0.0.1:7890"], "what listens on 7890")
    token = (user.home / ".local/state/tilr/token").read_text().strip()
    d = t.driver()
    d.connect("ws://127.0.0.1:7890", token=token)
    t.eq(d.value(d.shell_pane(), "$(id -un)"), user.name, "a pane over loopback")
    closed_from_peer(t)


@case(hosts=["peer"], spec=["daemon-lifecycle/tcp-port-defaults-to-loopback-7890"])
def a_bare_tilrd_listens_on_loopback_only(t: T):
    """UC-030: `tilrd` with no flags (as a service runs it): 127.0.0.1:7890 only."""
    dir = t.scratch("bare")
    log = dir / "log"
    t.spawn("tilrd", env=t.server_env(dir), log=log)
    t.eventually(lambda: "listening on ws://127.0.0.1:7890" in log.read_text(), "bare tilrd listens", timeout=15)
    t.eq(listeners(t), ["127.0.0.1:7890"], "what listens on 7890")
    closed_from_peer(t)
