"""UC-046: the connection breaks in the middle of a file write (FsWrite in chunks): the
file stays as it was, no temporary file is left, and the write goes through when retried.

The client writes 4 MB to ~/doc.txt in 64 chunks of 64 KiB, 40 ms apart, through the
layer-1 proxy. The server writes a temporary file next to the target
(.doc.txt.tilr-write-*) and renames it over the target after the last chunk. The break
is a reset (RST once 20 more messages went up; counted in messages because the protocol
compresses the chunks) or a silent freeze after a second (half-open: the server only
learns from its client timeout, 6 s here).
"""

import hashlib
import threading
import time
from pathlib import Path

from tt import T, case

CLIENT_TIMEOUT_S = 6
SIZE = 4 * 1024 * 1024
WRITE = {"path": "~/doc.txt", "size": SIZE, "fill": "new content\n", "chunk": 64 * 1024, "pause_ms": 40,
         "timeout_ms": 30_000}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def temps(home: Path) -> list[str]:
    return sorted(p.name for p in home.glob(".doc.txt.tilr-write-*"))


def prepare(t: T):
    server = t.server("--client-timeout", CLIENT_TIMEOUT_S, state=t.root / "state")
    home = Path(server.env["HOME"])
    target = home / "doc.txt"
    target.write_text("the original\n" * 1000)
    before = digest(target)
    proxy = t.proxy(server.port)
    client = t.driver("client")
    client.connect(proxy.url, token=server.token(), features=["ping", "files"])
    client.cmd("keepalive", interval_ms=1000, timeout_ms=4000, auto_reconnect=True, reconnect_timeout_ms=60_000)
    seen_temp = []

    def watch():
        while not seen_temp and not stop.is_set():
            seen_temp.extend(temps(home))
            time.sleep(0.01)
    stop = threading.Event()
    threading.Thread(target=watch, daemon=True).start()
    return server, proxy, client, home, target, before, seen_temp, stop


def after_break(t: T, client, home: Path, target: Path, before: str, seen_temp: list, stop, broke: float,
                limit: float):
    stop.set()
    t.ok(seen_temp, f"the write had a temporary file going ({seen_temp[:1]})")
    t.eventually(lambda: not temps(home), lambda: f"the temporary file is removed (left: {temps(home)})",
                 timeout=limit, interval=0.05)
    t.metric("temp_removed_ms", round((time.monotonic() - broke) * 1000))
    t.eq(digest(target), before, "the file is as it was")
    t.eventually(lambda: client.connected(), "the client is back", timeout=20)
    started = time.monotonic()
    ack = client.cmd("fs_write", **WRITE)
    t.metric("retry_ms", round((time.monotonic() - started) * 1000))
    t.eq(ack["error"], "", "the retried write succeeds")
    expected = hashlib.sha256((b"new content\n" * (SIZE // 12 + 1))[:SIZE]).hexdigest()
    t.eq(digest(target), expected, "the file has the new content")
    t.eq(temps(home), [], "no temporary file after the retry")


@case(fixture="server_state", spec=["files/an-abandoned-write-leaves-the-file-and-no-temporary-file"])
def a_write_cut_by_a_reset_leaves_the_file_as_it_was_and_no_temporary_file(t: T):
    """UC-046: RST after 20 of 64 chunks of a 4 MB write; the file unchanged, the temp file gone, the retry succeeds."""
    server, proxy, client, home, target, before, seen_temp, stop = prepare(t)
    proxy.set(dir="up", cut_after_ws_messages=20)
    answer = client.cmd("fs_write", ok=False, **WRITE)
    broke = time.monotonic()
    t.eq(answer["ev"], "error", f"the write fails on the client ({answer.get('message')})")
    after_break(t, client, home, target, before, seen_temp, stop, broke, limit=3)


@case(fixture="server_state", spec=["files/an-abandoned-write-leaves-the-file-and-no-temporary-file"])
def a_write_cut_by_a_silent_network_leaves_the_file_as_it_was_and_no_temporary_file(t: T):
    """UC-046: half-open after 1 s of a 4 MB write; once the server drops the client, the file unchanged, no temp file."""
    server, proxy, client, home, target, before, seen_temp, stop = prepare(t)
    timer = threading.Timer(1.0, lambda: proxy.freeze(conn="open"))
    timer.start()
    answer = client.cmd("fs_write", ok=False, **WRITE)
    timer.join()
    t.eq(answer["ev"], "error", f"the write fails on the client ({answer.get('message')})")
    frozen = proxy.events[-1][0]
    after_break(t, client, home, target, before, seen_temp, stop, frozen, limit=CLIENT_TIMEOUT_S + 3)
    proxy.resume(conn=1)
