"""UC-012: several `tilr` started at once on an empty machine: exactly one daemon.

The socket's lock decides: a tilr that finds no server takes the lock and hands it to the
tilrd it starts; the others wait for that one's socket. No second tilrd is even started
(its log would say "another tilrd").
"""

import os
import subprocess
import threading
from pathlib import Path

from tt import CLIENT_EXE, T, case


def one_daemon(t: T, env: dict):
    daemons = t.eventually(t.tilrd_daemons, "a tilrd runs")
    t.eq(len(daemons), 1, "exactly one tilrd")
    log = (Path(env["XDG_RUNTIME_DIR"]) / "tilr" / "tilrd.log").read_text()
    t.lacks(log, "another tilrd")
    t.eq(log.count("Unix socket for local clients"), 1, "one server ever took the socket")


@case()
def tilr_started_eight_times_at_once_starts_one_daemon(t: T):
    """UC-012: eight `tilr new -d` at once on an empty machine: eight panes, one tilrd."""
    t.need("procfs")
    env = t.user_env()
    results: list[subprocess.CompletedProcess] = []
    barrier = threading.Barrier(8)

    def one():
        barrier.wait()
        results.append(subprocess.run([CLIENT_EXE, "new", "-d"], env={**os.environ, **env}, capture_output=True,
                                      text=True, stdin=subprocess.DEVNULL, timeout=60))

    threads = [threading.Thread(target=one) for _ in range(8)]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    for r in results:
        t.eq(r.returncode, 0, f"tilr new -d: {r.stderr}")
    t.eq(len({r.stdout.strip() for r in results}), 8, "eight panes")
    one_daemon(t, env)


@case()
def two_terminals_running_tilr_at_once_share_one_daemon(t: T):
    """UC-012: `tilr` in two terminals at the same moment: both attach, one tilrd."""
    t.need("procfs")
    env = t.user_env()
    a = t.session("tilr", env=env, name="a")
    b = t.session("tilr", env=env, name="b")
    a.prompt(timeout=15)
    b.prompt(timeout=15)
    one_daemon(t, env)
