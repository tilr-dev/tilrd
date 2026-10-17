"""UC-017: `tilr kill-server`: the shells end, the socket and its lock go, no process of the
server is left (no orphans), and the attached clients end. (A program that ignores
SIGHUP is UC-086's.)
"""

from pathlib import Path

from tt import T, case, descendants


@case()
def kill_server_leaves_no_process_and_no_socket(t: T):
    """UC-017: kill-server with an attached client and three panes: nothing is left."""
    t.need("procfs")
    env = t.user_env()
    run = Path(env["XDG_RUNTIME_DIR"]) / "tilr"
    client = t.session("tilr", env=env, name="tilr")
    client.prompt(timeout=15)
    t.tilr("new", "-d", "--", "sleep", "1000", env=env, check=True)
    t.tilr("new", "-d", env=env, check=True)
    daemon = t.tilrd_daemons()[0]
    left = [p.pid for p in descendants(daemon.pid)]
    t.ok(len(left) >= 6, f"pane processes and what runs in them: {left}")
    t.ok((run / "tilrd.sock").exists() and (run / "tilrd.sock.lock").exists(), "socket and lock")

    t.tilr("kill-server", env=env, check=True)
    client.expect(r"\[tilrd stopped\]", timeout=10)
    t.eq(client.wait_exit(), 0, "the attached tilr ends")
    t.eq(t.tilrd_daemons(), [], "no tilrd")
    t.eventually(lambda: [pid for pid in [daemon.pid, *left] if (p := t.proc(pid)) and p.alive] == [],
                 lambda: "no process of the server is left: " +
                 str([t.proc(pid) for pid in [daemon.pid, *left] if t.proc(pid) and t.proc(pid).alive]),
                 timeout=10)
    t.eq(sorted(p.name for p in run.iterdir() if p.name.startswith("tilrd.sock")), [], "socket and lock removed")
    r = t.tilr("ls", env=env)
    t.ok(r.code != 0 and "no tilrd runs" in r.err, f"nothing answers: {r.text}")
