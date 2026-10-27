"""UC-074: SOCKS through the client: a program on the user's machine (curl) reaches a
service that listens only on the server's loopback, through the client's local SOCKS
port and the server's /socks WebSocket; several sessions at once; a big response.

The user's machine is the host "laptop", a container of its own: tilr-driver there
connects to the server (listening on 0.0.0.0, the case's container) and opens a SOCKS5
port on the laptop's 127.0.0.1 (`socks_listen`), each of whose connections it carries to
/socks as they are. curl on the laptop uses it (`--socks5-hostname`: names resolved on
the server, so `localhost` is the server's). The service is an HTTP server on the
server's 127.0.0.1, which the laptop cannot reach directly.
"""

import functools
import hashlib
import http.server
import os
import threading

from tt import T, case


def http_service(t: T):
    """An HTTP server on this container's loopback, serving a directory: index.html,
    20 files of 1 MiB and one of 100 MiB, all random but index.html."""
    root = t.scratch("www")
    (root / "index.html").write_text("<h1>only on the server's loopback</h1>\n")
    for n in range(20):
        (root / f"part{n}.bin").write_bytes(os.urandom(1 << 20))
    (root / "big.bin").write_bytes(os.urandom(100 << 20))

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    t._cleanup.append(httpd.shutdown)
    return root, httpd.server_address[1]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def setup(t: T):
    root, port = http_service(t)
    server = t.server("--socks", listen="0.0.0.0:0", state=t.root / "state")
    laptop = t.host("laptop")
    d = laptop.driver("client")
    d.connect(f"ws://{laptop.peer_ip}:{server.port}", token=server.token())
    socks = d.cmd("socks_listen")["port"]
    return root, port, server, laptop, d, socks


def curl(laptop, socks: int, url: str, out: str = "-", timeout: float = 60):
    return laptop.run("curl", "-sS", "--fail", "--max-time", str(int(timeout)), "--socks5-hostname",
                      f"127.0.0.1:{socks}", "-o", out, url, timeout=timeout + 10, quiet=out != "-")


@case(fixture="server_state", hosts=["laptop"], spec=[
    "socks-proxy/names-are-resolved-on-the-server-and-localhost-is-its-loopback",
])
def curl_on_the_clients_machine_reaches_a_service_on_the_servers_loopback(t: T):
    """UC-074: curl via the client's SOCKS port gets a page served only on the server's 127.0.0.1 (and localhost)."""
    root, port, server, laptop, d, socks = setup(t)
    direct = laptop.run("curl", "-sS", "--max-time", "5", f"http://{laptop.peer_ip}:{port}/index.html")
    t.ok(direct.code != 0, "the laptop cannot reach the service directly")
    for host in ("127.0.0.1", "localhost"):
        r = curl(laptop, socks, f"http://{host}:{port}/index.html")
        t.eq((r.code, r.out), (0, (root / "index.html").read_text()), f"the page through SOCKS ({host})")
    stats = d.cmd("socks_stats")
    t.eq((stats["sessions"], stats["failed"]), (2, 0), "two SOCKS sessions went through the client")


@case(fixture="server_state", hosts=["laptop"], spec=["socks-proxy/sessions-run-side-by-side-and-with-the-terminal"])
def twenty_sessions_at_once_each_get_their_own_file(t: T):
    """UC-074: 20 curls at once through the client: each file arrives whole (sha256)."""
    root, port, server, laptop, d, socks = setup(t)
    script = (f"cd /tmp && seq 0 19 | xargs -P 20 -I N curl -sS --fail --max-time 60 "
              f"--socks5-hostname 127.0.0.1:{socks} -o partN.bin http://127.0.0.1:{port}/partN.bin && "
              "sha256sum part*.bin")
    r = laptop.sh(script, timeout=120)
    t.eq(r.code, 0, "every curl succeeds")
    got = dict(reversed(line.split()) for line in r.out.splitlines())
    want = {f"part{n}.bin": sha((root / f"part{n}.bin").read_bytes()) for n in range(20)}
    t.eq(got, want, "every file arrives whole")
    stats = d.cmd("socks_stats")
    t.eq((stats["sessions"], stats["failed"], stats["open"]), (20, 0, 0), "20 sessions, none failed, all closed")


@case(fixture="server_state", hosts=["laptop"], timeout=400, spec=[
    "socks-proxy/sessions-run-side-by-side-and-with-the-terminal",
])
def a_big_response_through_socks_arrives_whole(t: T):
    """UC-074: 100 MiB through the client's SOCKS port: hash equal; throughput recorded."""
    root, port, server, laptop, d, socks = setup(t)
    r = laptop.sh(f"cd /tmp && a=$(date +%s.%N); curl -sS --fail --max-time 300 --socks5-hostname "
                  f"127.0.0.1:{socks} -o big.bin http://127.0.0.1:{port}/big.bin; s=$?; b=$(date +%s.%N); "
                  f"echo $s; python3 -c \"print($b - $a)\"; "
                  f"sha256sum big.bin", timeout=330)
    lines = r.out.split()
    t.eq(lines[0], "0", f"curl succeeds ({r.text[-300:]})")
    seconds = float(lines[1])
    t.metric("socks_mb_s", round(100 * 1.048576 / max(seconds, 0.001), 1))
    t.eq(lines[2], sha((root / "big.bin").read_bytes()), "the 100 MiB arrive whole")
