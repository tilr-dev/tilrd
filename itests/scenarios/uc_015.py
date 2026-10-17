"""UC-015: what is at the socket's path is not a tilrd's: another program listens there,
or a file that is not a socket is in the way. `tilr` (and tilrd) say so clearly and
remove nothing; no tilrd is left running.
"""

import socket as sockets
import threading
from pathlib import Path

from tt import SERVER_EXE, T, case


def socket_path(env: dict) -> Path:
    path = Path(env["XDG_RUNTIME_DIR"]) / "tilr" / "tilrd.sock"
    path.parent.mkdir(mode=0o700)
    return path


@case()
def another_programs_listener_at_the_socket_is_left_alone(t: T):
    """UC-015: a program that is not tilrd listens on the socket: a clear error, it keeps listening."""
    t.need("procfs")
    env = t.user_env()
    path = socket_path(env)
    listener = sockets.socket(sockets.AF_UNIX, sockets.SOCK_STREAM)
    listener.bind(str(path))
    listener.listen(16)
    accepted = []

    def serve():
        while True:
            try:
                conn, _ = listener.accept()
            except OSError:
                return
            accepted.append(1)
            try:
                conn.sendall(b"HTTP/1.1 418 I'm a teapot\r\n\r\n")
            except OSError:
                pass
            conn.close()

    threading.Thread(target=serve, daemon=True).start()

    r = t.tilr("new", "-d", env=env, timeout=30)
    t.ok(r.code != 0, "tilr fails")
    t.has(r.err, "something that is not tilrd listens on", str(path), "nothing was removed")
    r = t.run(SERVER_EXE, "--socket", path, env=env, timeout=30)
    t.ok(r.code != 0, "tilrd refuses it too")
    t.has(r.err, "something that is not a tilrd listens on", "nothing was removed")
    t.eq(t.tilrd_daemons(), [], "no tilrd is left running")
    before = len(accepted)
    probe = sockets.socket(sockets.AF_UNIX, sockets.SOCK_STREAM)
    probe.settimeout(5)
    probe.connect(str(path))
    t.eventually(lambda: len(accepted) > before, "the other program still has its socket")
    listener.close()


@case()
def a_file_that_is_not_a_socket_is_left_alone(t: T):
    """UC-015: a plain file at the socket's path: a clear error, the file as it was."""
    t.need("procfs")
    env = t.user_env()
    path = socket_path(env)
    path.write_text("not a socket\n")
    for argv in (["tilr", "new", "-d"], ["tilr"], [SERVER_EXE, "--socket", path]):
        r = t.run(*argv, env=env, timeout=30)
        t.ok(r.code != 0, f"{argv[0]} fails")
        t.has(r.err, str(path), "not a socket", "nothing was removed")
    t.eq(path.read_text(), "not a socket\n", "the file is as it was")
    t.eq(t.tilrd_daemons(), [], "no tilrd is left running")
