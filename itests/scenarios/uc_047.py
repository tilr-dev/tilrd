"""UC-047: the connection breaks in the middle of a stream (the SSH agent, SOCKS): the
program on the server's side gets the end of its stream, and the next stream works.

Agent: a program in a pane talks to $SSH_AUTH_SOCK, which the server streams to the
client that announced "agent" (tilr-driver answering as a toy agent that echoes). SOCKS:
the client opens /socks and connects through it to an echo service on the server's
machine. Both go through the layer-1 proxy; the break is a reset (RST) or a silent
freeze of the connection (half-open, noticed by the server's client timeout, 6 s here).
"""

import socket
import threading
import time

from tt import T, case

CLIENT_TIMEOUT_S = 6

# Talks to the agent until the stream ends: 64 bytes every 50 ms, each echoed back; a
# line of progress per echo, "EOF <bytes>" at the end. With a count, stops after that
# many echoes ("DONE").
AGENT_PROGRAM = r"""
import os, socket, sys, time
log = open(sys.argv[1], "a", buffering=1)
count = int(sys.argv[2]) if len(sys.argv) > 2 else 0
s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["SSH_AUTH_SOCK"])
s.settimeout(120)
got = n = 0
while True:
    n += 1
    msg = b"%06d" % n + b"." * 58
    try:
        s.sendall(msg)
        data = b""
        while len(data) < len(msg):
            chunk = s.recv(len(msg) - len(data))
            if not chunk:
                raise EOFError
            data += chunk
    except (EOFError, OSError) as e:
        log.write(f"EOF {got} {time.time():.3f} {type(e).__name__}\n")
        break
    got += len(data)
    log.write(f"echo {n} {data == msg}\n")
    if count and n >= count:
        log.write("DONE\n")
        break
    time.sleep(0.05)
"""


def agent_setup(t: T):
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, state=t.root / "state")
    proxy = t.proxy(server.port)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token(), features=["agent"])
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True, reconnect_timeout_ms=60_000)
    pane = client.shell_pane(100, 30)
    program = t.root / "agent_program.py"
    program.write_text(AGENT_PROGRAM)
    return server, proxy, client, pane, program


def echoes(log) -> list[str]:
    return log.read_text().splitlines() if log.exists() else []


def agent_case(t: T, cut, limit: float):
    server, proxy, client, pane, program = agent_setup(t)
    log = t.scratch("agent") / "first"
    client.line(pane, f"python3 {program} {log}")
    client.wait_event("stream_open", timeout=10)
    t.eventually(lambda: len(echoes(log)) >= 20, "the program talks to the agent", timeout=10)
    t.ok(all(line.endswith("True") for line in echoes(log)), "every echo came back intact")
    broke = time.monotonic()
    cut(proxy)
    t.eventually(lambda: any(line.startswith("EOF") for line in echoes(log)), "the program reads the end of its stream",
                 timeout=limit, interval=0.05)
    t.metric("program_eof_ms", round((time.monotonic() - broke) * 1000))
    t.eventually(lambda: client.connected(), "the client is back", timeout=20)
    client.wait_for(pane, r"(?m)^\$ ?$", timeout=10)
    again = t.scratch("agent") / "second"
    t.eq(client.run(pane, f"python3 {program} {again} 20", timeout=30), 0, "the next agent stream runs")
    lines = echoes(again)
    t.eq(lines[-1], "DONE", "the next stream got all its echoes")
    t.ok(all(line.endswith("True") for line in lines[:-1]), "every echo of the next stream came back intact")


@case(fixture="server_state", spec=["ssh-agent-forwarding/a-client-that-goes-takes-its-streams-with-it"])
def an_agent_stream_cut_by_a_reset_ends_for_the_program_and_the_next_one_works(t: T):
    """UC-047: RST in the middle of an agent stream; the program reads EOF at once, the next stream works."""
    agent_case(t, lambda proxy: proxy.cut(), limit=3)


@case(fixture="server_state", spec=["ssh-agent-forwarding/a-client-that-goes-takes-its-streams-with-it"])
def an_agent_stream_on_a_silent_connection_ends_for_the_program_and_the_next_one_works(t: T):
    """UC-047: half-open in the middle of an agent stream; the program reads EOF once the server drops the client."""
    agent_case(t, lambda proxy: proxy.freeze(conn="open"), limit=CLIENT_TIMEOUT_S + 3)


class EchoService:
    """An echo service on the server's machine (this container), noting when each
    connection ends."""

    def __init__(self):
        self.sock = socket.create_server(("127.0.0.1", 0))
        self.port = self.sock.getsockname()[1]
        self.ended: list[float] = []
        self.opened = 0
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            conn, _ = self.sock.accept()
            self.opened += 1
            threading.Thread(target=self._serve, args=(conn,), daemon=True).start()

    def _serve(self, conn):
        try:
            while data := conn.recv(65536):
                conn.sendall(data)
        except OSError:
            pass
        self.ended.append(time.monotonic())
        conn.close()


def socks_case(t: T, cut, limit: float):
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, "--socks", state=t.root / "state")
    proxy = t.proxy(server.port)
    echo = EchoService()
    client = t.driver("client")
    client.connect(proxy.url, token=server.token())
    sid = client.cmd("socks_open", host="127.0.0.1", port=echo.port)["socks"]
    wrong = []
    for n in range(20):
        client.cmd("socks_send", socks=sid, text=f"ping {n}\n", quiet=True)
        got = client.cmd("socks_recv", socks=sid, bytes=len(f"ping {n}\n"), quiet=True)
        if got["text"] != f"ping {n}\n":
            wrong.append(got["text"])
    t.eq(wrong, [], "every message through SOCKS came back")
    socks_conn = max(s["n"] for s in proxy.stats())
    broke = time.monotonic()
    cut(proxy, socks_conn)
    t.eventually(lambda: echo.ended, "the service on the server's machine sees its connection end", timeout=limit,
                 interval=0.05)
    t.metric("service_eof_ms", round((echo.ended[0] - broke) * 1000))
    sid2 = client.cmd("socks_open", host="127.0.0.1", port=echo.port)["socks"]
    client.cmd("socks_send", socks=sid2, size=200_000, fill="next stream ")
    got = client.cmd("socks_recv", socks=sid2, bytes=200_000, timeout_ms=10_000)
    t.eq(got["received"], 200_000, "the next SOCKS stream echoes everything")
    t.eq(echo.opened, 2, "two connections reached the service")


@case(fixture="server_state", spec=["socks-proxy/a-session-ends-with-its-connection-and-with-the-server"])
def a_socks_stream_cut_by_a_reset_ends_for_the_service_and_the_next_one_works(t: T):
    """UC-047: RST in the middle of a SOCKS session; the service sees the end at once, the next session works."""
    socks_case(t, lambda proxy, conn: proxy.cut(conn=conn), limit=3)


@case(fixture="server_state", spec=["socks-proxy/a-silent-socks-websocket-is-dropped"])
def a_socks_stream_on_a_silent_connection_ends_for_the_service_and_the_next_one_works(t: T):
    """UC-047: half-open SOCKS session; the service sees the end once the server drops the silent WebSocket."""
    socks_case(t, lambda proxy, conn: proxy.freeze(conn=conn), limit=CLIENT_TIMEOUT_S + 3)
