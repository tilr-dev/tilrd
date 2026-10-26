"""UC-032: IPv6: `--listen [::1]:PORT` serves IPv6 loopback only (127.0.0.1 refused);
`--listen [::]:PORT` serves both ::1 and 127.0.0.1 (dual stack). The Host header of a
client on [::1] is taken as loopback.
"""

import socket

from tt import Blocked, T, case


def need_ipv6(t: T):
    if "::1" not in t.run("ip", "-6", "addr", "show", "lo", quiet=True).out:
        raise Blocked("no IPv6 loopback in this container")


def shell(t: T, url: str, token: str, name: str):
    d = t.driver(name)
    d.connect(url, token=token)
    t.eq(d.value(d.shell_pane(), "$((6*7))"), "42", f"a shell over {url}")


@case(spec=["daemon-lifecycle/tcp-port-defaults-to-loopback-7890"])
def ipv6_loopback_serves_ipv6_only(t: T):
    """UC-032: --listen [::1]: a client on [::1] gets a shell; 127.0.0.1 is refused."""
    need_ipv6(t)
    server = t.server(listen="[::1]:0")
    t.has(server.url, "[::1]")
    shell(t, server.url, server.token(), "v6")
    try:
        socket.create_connection(("127.0.0.1", server.port), timeout=3).close()
        t.ok(False, "127.0.0.1 is not served")
    except ConnectionRefusedError:
        print("  ok: 127.0.0.1 refused")


@case(spec=["daemon-lifecycle/tcp-port-defaults-to-loopback-7890"])
def dual_stack_serves_both_loopbacks(t: T):
    """UC-032: --listen [::]: shells over [::1] and over 127.0.0.1."""
    need_ipv6(t)
    server = t.server(listen="[::]:0")
    shell(t, f"ws://[::1]:{server.port}", server.token(), "v6")
    shell(t, f"ws://127.0.0.1:{server.port}", server.token(), "v4")
