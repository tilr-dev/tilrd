"""UC-073: data over a stream (StreamOpen / StreamData / StreamClose): throughput, byte
order, a big stream, and back pressure when the receiving side is slow.

The stream is the one kind there is: a program in a pane connects to $SSH_AUTH_SOCK and
the server carries its bytes to the client that announced "agent" (crates/tilrd/
src/agent.rs). The client is tilr-driver: echoing (a toy agent), taking and hashing
(sink), or sending (stream_send). The program (STREAMER below) sends random bytes, reads
them back, or reads slowly, and writes what it did as JSON.

Back pressure, as agent.rs promises it: "the socket is read only as fast as the client's
own queue takes packets, so a slow client holds the program up rather than memory
growing"; the other way, a client that runs more than about 4 MiB ahead of the program
loses the stream. A slow client is one behind the layer-1 proxy limited to 1 MB/s down;
it keeps reading, answering pings, and watching a counter, so it must not be dropped.

Was red until 2026-10-17: the slow client's queue was counted in packets, so 64 MiB were
buffered for it and the program never held back. Now stream data goes by the client's
bulk lane, which holds at most 4 MiB (crates/tilrd/src/outbox.rs BULK_BYTES), and
the agent's pump waits for room in it.
"""

import json
import time
from pathlib import Path

from tt import T, Counter, case

# streamer.py MODE ARGS... OUT
#   echo SIZE OUT   send SIZE random bytes (64 KiB writes, a thread) and read them back
#   send SIZE OUT   send SIZE random bytes, then close; progress every 0.25 s
#   recv RATE OUT   read at RATE bytes/s until EOF
STREAMER = r"""
import hashlib, json, os, socket, sys, threading, time
mode, out = sys.argv[1], sys.argv[-1]
s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["SSH_AUTH_SOCK"])
started = time.monotonic()
result = {"mode": mode}
def save():
    result["seconds"] = round(time.monotonic() - started, 3)
    with open(out + ".tmp", "w") as f:
        json.dump(result, f)
    os.replace(out + ".tmp", out)
if mode in ("echo", "send"):
    size = int(sys.argv[2])
    sent_sha, progress = hashlib.sha256(), []
    result["progress"] = progress
    def sender():
        sent, last = 0, 0.0
        while sent < size:
            block = os.urandom(min(65536, size - sent))
            s.sendall(block)
            sent_sha.update(block)
            sent += len(block)
            now = time.monotonic() - started
            if now - last >= 0.25:
                progress.append([round(now, 2), sent])
                last = now
                if mode == "send":
                    save()
        result["sent"] = sent
        result["sent_sha256"] = sent_sha.hexdigest()
        result["sent_s"] = round(time.monotonic() - started, 3)
    if mode == "send":
        sender()
        s.close()
    else:
        thread = threading.Thread(target=sender)
        thread.start()
        got, recv_sha = 0, hashlib.sha256()
        while got < size:
            chunk = s.recv(65536)
            if not chunk:
                result["eof_early"] = True
                break
            recv_sha.update(chunk)
            got += len(chunk)
        thread.join()
        result["received"] = got
        result["received_sha256"] = recv_sha.hexdigest()
elif mode == "recv":
    rate = int(sys.argv[2])
    got, recv_sha = 0, hashlib.sha256()
    while True:
        chunk = s.recv(16384)
        if not chunk:
            break
        recv_sha.update(chunk)
        got += len(chunk)
        result["received"] = got
        time.sleep(len(chunk) / rate)
    result["received"] = got
    result["received_sha256"] = recv_sha.hexdigest()
    result["eof"] = True
result["done"] = True
save()
"""


def start(t: T, proxy: bool = False, server_args=()):
    server = t.server(*server_args, state=t.root / "state")
    p = None
    d = t.driver()
    if proxy:
        p = t.proxy(server.port)
        d.connect(p.url, token=server.token(), features=["agent"])
    else:
        d.connect(server, features=["agent"])
    pane = d.shell_pane()
    program = t.root / "streamer.py"
    program.write_text(STREAMER)
    return server, p, d, pane, program


def result(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return {}


@case(fixture="server_state", spec=["ssh-agent-forwarding/bytes-go-both-ways-unchanged-and-in-order"])
def a_stream_echoed_by_the_client_comes_back_whole_in_order(t: T):
    """UC-073: 64 MiB to the client and back through one stream: same hash both ways; throughput recorded."""
    server, _, d, pane, program = start(t)
    d.cmd("agent", mode="echo")
    out = t.scratch("stream") / "echo.json"
    size = 64 << 20
    d.line(pane, f"python3 {program} echo {size} {out}")
    r = t.eventually(lambda: result(out).get("done") and result(out), "the program is done", timeout=180)
    t.eq((r["sent"], r["received"]), (size, size), "bytes sent and received back")
    t.eq(r["received_sha256"], r["sent_sha256"], "what came back is what was sent, in order")
    t.metric("echo_mb_s", round(2 * size / 1e6 / r["seconds"], 1))


@case(fixture="server_state", timeout=400, spec=["ssh-agent-forwarding/bytes-go-both-ways-unchanged-and-in-order"])
def a_big_stream_to_the_client_arrives_whole_and_in_order(t: T):
    """UC-073: 256 MiB from a program to the client: the client's hash of the stream equals the program's."""
    server, _, d, pane, program = start(t)
    d.cmd("agent", mode="sink")
    out = t.scratch("stream") / "send.json"
    size = 256 << 20
    since = d.mark()
    d.line(pane, f"python3 {program} send {size} {out}")
    closed = d.wait_event("stream_closed", since=since, timeout=300)
    # The program writes its result after it closed the stream: the client may hear of the
    # close first.
    r = t.eventually(lambda: result(out).get("done") and result(out), "the program is done", timeout=30)
    t.eq(r.get("sent"), size, "the program sent it all")
    t.eq((closed["bytes_in"], closed["sha256_in"]), (size, r["sent_sha256"]),
         "the client got every byte, in order (sha256)")
    t.metric("stream_mb_s", round(size / 1e6 / r["seconds"], 1))


@case(fixture="server_state", spec=[
    "client-liveness/frames-go-ahead-of-data-in-quantity",
    "ssh-agent-forwarding/a-slow-client-holds-the-program-back",
])
def a_slow_client_holds_the_program_back_and_is_neither_dropped_nor_buffered_for(t: T):
    """UC-073: the client reads at 1 MB/s: the program is held back, server memory flat, the client stays; then all of it arrives."""
    server, proxy, d, pane, program = start(t, proxy=True)
    d.cmd("agent", mode="sink")
    counter_pane = d.shell_pane()
    counter = Counter(t, d, counter_pane, period=0.2)
    d.wait_for(counter_pane, r"(?m)^tick 3$")
    rss0 = t.proc(server.pid).rss_kb
    proxy.set(dir="down", rate_bps=1_000_000)
    out = t.scratch("stream") / "send.json"
    size = 64 << 20
    since = d.mark()
    d.line(pane, f"python3 {program} send {size} {out}")
    peak = 0
    for _ in range(40):
        time.sleep(0.25)
        peak = max(peak, t.proc(server.pid).rss_kb - rss0)
    r = result(out)
    sent = r.get("sent") or (r.get("progress") or [[0, 0]])[-1][1]
    t.metric("program_sent_after_10s_mb", round(sent / 1e6, 1))
    t.metric("server_rss_growth_kb", peak)
    dropped = [e for e in d.events[since:] if e.get("ev") == "disconnected"]
    t.check("the slow client is not dropped", lambda: t.eq(dropped, [], "disconnects"))
    t.check("the program is held back to about the client's pace",
            lambda: t.ok(sent <= 24 << 20, f"in 10 s at 1 MB/s the program got {sent / 1e6:.1f} MB out "
                                           f"(at most 24 MiB: the link, the socket buffers and a few queued packets)"))
    t.check("the server's memory does not grow by what the client is behind",
            lambda: t.ok(peak <= 16 * 1024, f"server RSS grew by at most 16 MB (+{peak} kB)"))
    proxy.set(dir="down", rate_bps=0)

    def all_arrives():
        closed = d.wait_event("stream_closed", since=since, timeout=120)
        r = t.eventually(lambda: result(out).get("done") and result(out), "the program is done", timeout=30)
        t.eq((closed["bytes_in"], closed["sha256_in"]), (size, r["sent_sha256"]), "everything, in order")
    t.check("once the client is fast again the whole stream arrives in order", all_arrives)
    t.check("the counter kept going on the slow client's screen", lambda: counter.check_screen(d))


@case(fixture="server_state", spec=["ssh-agent-forwarding/a-client-running-ahead-of-the-program-loses-the-stream"])
def a_client_running_ahead_of_a_slow_program_is_held_to_the_queue_and_then_loses_the_stream(t: T):
    """UC-073: 2 MiB to a program reading 512 KB/s arrives whole; 64 MiB at once ends the stream, memory flat."""
    server, _, d, pane, program = start(t)
    d.cmd("agent", mode="sink")
    out = t.scratch("stream") / "recv.json"
    since = d.mark()
    d.line(pane, f"python3 {program} recv 512000 {out}")
    stream = d.wait_event("stream_open", since=since, timeout=10)["stream"]
    sent = d.cmd("stream_send", stream=stream, size=2 << 20, fill="0123456789abcdef-slow-reader\n")
    d.cmd("stream_close", stream=stream)
    r = t.eventually(lambda: result(out).get("done") and result(out), "the slow program reads it all", timeout=30)
    t.eq((r["received"], r["received_sha256"]), (2 << 20, sent["sha256"]), "2 MiB within the queue arrive whole")

    out2 = t.scratch("stream") / "recv2.json"
    since = d.mark()
    d.line(pane, f"python3 {program} recv 100000 {out2}")
    stream = d.wait_event("stream_open", since=since, timeout=10)["stream"]
    rss0 = t.proc(server.pid).rss_kb
    d.cmd("stream_send", stream=stream, size=64 << 20, timeout=120)
    peak = 0
    for _ in range(20):
        time.sleep(0.25)
        peak = max(peak, t.proc(server.pid).rss_kb - rss0)
    t.metric("server_rss_growth_kb", peak)
    t.check("the server tells the client the stream is over",
            lambda: d.wait_event("stream_closed", since=since, timeout=10))
    t.check("the program reads the end of its stream",
            lambda: t.eventually(lambda: result(out2).get("eof"), "EOF", timeout=120))
    t.check("the program got no more than the queue held",
            lambda: t.ok(result(out2).get("received", 0) <= 8 << 20,
                         f"received {result(out2).get('received')} bytes (at most 8 MiB)"))
    t.check("the server's memory does not grow by what the client sent ahead",
            lambda: t.ok(peak <= 16 * 1024, f"server RSS grew by at most 16 MB (+{peak} kB)"))
    t.check("the client is still connected", lambda: t.ok(d.connected(), "connected"))
