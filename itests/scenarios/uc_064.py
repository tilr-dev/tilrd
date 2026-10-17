"""UC-064: an update while a file is being written, a SOCKS session is open and a program
talks to the forwarded agent. A reload does not carry client connections over, so each
of these ends with it; what the use case asks is that it ends cleanly and the next one
works on the new server:

* a file write (FsWrite in chunks) broken by the reload leaves the file as it was and no
  temporary file next to it, and the same write, retried, goes through;
* a SOCKS session ends for the service on the server's machine (it reads EOF, it is not
  left hanging), and a new session works;
* a program talking to $SSH_AUTH_SOCK reads the end of its stream, and the next agent
  stream (the socket made again at the same path) works.

The update is the new executable renamed over the installed one, as `tilrd
update` finishes; the server notices it and reloads (auto reload). The panes' shells
survive all of it.

Was red until 2026-10-17: the write cut by the reload left its temporary file
(.doc.txt.tilr-write-*) for good, its task dying with the exec. Now each write notes its
temporary file in `writes/` of the runtime directory, and a server that starts or has just
reloaded removes those of writes whose process is gone or was itself
(crates/tilrd/src/files.rs `note_write`, `sweep_writes`).
"""

import hashlib
import shutil
import socket
import threading
import time
from pathlib import Path

from tt import NEXT_EXE, T, case
from tt.live import wait_reloads

WRITE = {"path": "~/doc.txt", "size": 8 * 1024 * 1024, "fill": "new content\n", "chunk": 64 * 1024,
         "pause_ms": 50, "timeout_ms": 60_000}


def install_next(exe: Path):
    """What `tilrd update` does last: the new executable renamed over the old."""
    staged = exe.with_name(exe.name + ".new.tt")
    shutil.copy2(NEXT_EXE, staged)
    staged.rename(exe)


def start(t: T, *args, features=("ping",)):
    exe = t.install()
    server = t.server(*args, exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server, features=list(features))
    d.cmd("keepalive", interval_ms=500, timeout_ms=3000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    shell = d.shell_pane()
    d.line(shell, "export MARK=kept")
    return exe, server, d, shell


def after(t: T, server, d, shell, since: int):
    wait_reloads(t, server, 1, timeout=30)
    t.eq(d.wait_event("reconnected", since=since, timeout=30)["server_version"], t.manifest()["next_version"],
         "the client is back on the new version")
    t.check("the shell survived the update", lambda: t.eq(d.value(shell, "$MARK"), "kept", "the variable"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@case(fixture="server_state")
def a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works(t: T):
    """UC-064: an 8 MB write in chunks while the server updates: the file unchanged, no temp file, the retry succeeds."""
    exe, server, d, shell = start(t, features=("ping", "files"))
    home = Path(server.env["HOME"])
    target = home / "doc.txt"
    target.write_text("the original\n" * 1000)
    before = digest(target)
    timer = threading.Timer(1.0, install_next, args=(exe,))
    since = d.mark()
    timer.start()
    answer = d.cmd("fs_write", ok=False, **WRITE)
    timer.join()
    t.eq(answer["ev"], "error", f"the write broken by the reload fails on the client ({answer.get('message')})")
    after(t, server, d, shell, since)
    temps = lambda: sorted(p.name for p in home.glob(".doc.txt.tilr-write-*"))
    t.check("no temporary file is left", lambda: t.eventually(lambda: not temps(), lambda: f"left: {temps()}",
                                                               timeout=5))
    t.check("the file is as it was", lambda: t.eq(digest(target), before, "content"))
    ack = d.cmd("fs_write", **WRITE)
    t.eq(ack["error"], "", "the retried write succeeds")
    expected = hashlib.sha256((b"new content\n" * (WRITE["size"] // 12 + 1))[:WRITE["size"]]).hexdigest()
    t.eq(digest(target), expected, "the file has the new content")


class EchoService:
    """An echo service on the server's machine, noting when each connection ends."""

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


@case(fixture="server_state")
def a_socks_session_open_during_an_update_ends_for_the_service_and_a_new_one_works(t: T):
    """UC-064: a SOCKS session open through an update: the service sees its end, a new session echoes 1 MB."""
    exe, server, d, shell = start(t, "--socks")
    echo = EchoService()
    sid = d.cmd("socks_open", host="127.0.0.1", port=echo.port)["socks"]
    d.cmd("socks_send", socks=sid, text="before\n")
    t.eq(d.cmd("socks_recv", socks=sid, bytes=7)["text"], "before\n", "the session echoes before the update")
    since = d.mark()
    installed = time.monotonic()
    install_next(exe)
    after(t, server, d, shell, since)
    t.eventually(lambda: echo.ended, "the service on the server's machine sees its connection end", timeout=10)
    t.metric("service_eof_after_install_ms", round((echo.ended[0] - installed) * 1000))
    got = d.cmd("socks_recv", socks=sid, bytes=1, timeout_ms=3000)
    t.ok(got["ended"], f"the client's side of the old session ended too ({got['ended']})")
    sid2 = d.cmd("socks_open", host="127.0.0.1", port=echo.port)["socks"]
    d.cmd("socks_send", socks=sid2, size=1_000_000, fill="after the update ")
    got = d.cmd("socks_recv", socks=sid2, bytes=1_000_000, timeout_ms=10_000)
    t.eq(got["received"], 1_000_000, "a new SOCKS session echoes everything")


# Talks to the agent until its stream ends (as in UC-047): 64 bytes every 50 ms, each
# echoed back; "EOF ..." at the end; with a count, "DONE" after that many.
AGENT_PROGRAM = r"""
import os, socket, sys, time
log = open(sys.argv[1], "a", buffering=1)
count = int(sys.argv[2]) if len(sys.argv) > 2 else 0
s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["SSH_AUTH_SOCK"])
s.settimeout(60)
n = 0
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
        log.write(f"EOF {time.time():.3f} {type(e).__name__}\n")
        break
    log.write(f"echo {n} {data == msg}\n")
    if count and n >= count:
        log.write("DONE\n")
        break
    time.sleep(0.05)
"""


@case(fixture="server_state")
def an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works(t: T):
    """UC-064: a program talking to the agent through an update reads EOF; the next agent stream works."""
    exe, server, d, shell = start(t, features=("ping", "agent"))
    program = t.root / "agent_program.py"
    program.write_text(AGENT_PROGRAM)
    pane = d.shell_pane()
    log = t.scratch("agent") / "first"
    d.line(pane, f"python3 {program} {log}")
    lines = lambda path: path.read_text().splitlines() if path.exists() else []
    t.eventually(lambda: len(lines(log)) >= 10, "the program talks to the agent", timeout=10)
    since = d.mark()
    installed = time.monotonic()
    install_next(exe)
    t.eventually(lambda: any(l.startswith("EOF") for l in lines(log)), "the program reads the end of its stream",
                 timeout=15, interval=0.05)
    t.metric("program_eof_after_install_ms", round((time.monotonic() - installed) * 1000))
    after(t, server, d, shell, since)
    d.wait_for(pane, r"(?m)^\$ ?$", timeout=10)
    again = t.scratch("agent") / "second"
    t.eq(d.run(pane, f"python3 {program} {again} 20", timeout=30), 0, "the next agent stream runs")
    t.eq(lines(again)[-1], "DONE", "the next stream got all its echoes")
    t.ok(all(l.endswith("True") for l in lines(again)[:-1]), "every echo of the next stream came back intact")
