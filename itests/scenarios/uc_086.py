"""UC-086: a program in a pane that ignores SIGHUP does not outlive the server. On a stop
each pane process sends SIGHUP to its shell's process group and to the pty's foreground
group and SIGKILLs what is left of them 2 s later, and the server waits for its pane
processes; a `nohup` job in the background is left, as a terminal hangup leaves it.
"""

import os
import signal

from tt import T, case, descendants


@case(spec=[
    "daemon-lifecycle/no-shell-outlives-the-server#shell-ignoring-sighup",
    "daemon-lifecycle/no-shell-outlives-the-server#foreground-job-ignoring-sighup",
    "daemon-lifecycle/no-shell-outlives-the-server#nohup-job",
])
def a_shell_ignoring_sighup_does_not_outlive_the_server(t: T):
    """UC-086: kill-server ends a pane program and a foreground job that ignore SIGHUP; a nohup job is left."""
    t.need("procfs")
    env = t.user_env()
    pids = t.scratch("pids")

    def pid(name: str) -> int:
        try:
            return int((pids / name).read_text())
        except (OSError, ValueError):
            return 0

    def alive(n: int) -> bool:
        return bool(n) and (p := t.proc(n)) is not None and p.alive

    # An interactive shell: a nohup job in the background, then a foreground job that
    # ignores SIGHUP.
    client = t.session("tilr", env=env, name="tilr")
    client.prompt(timeout=15)
    # And a pane whose own program ignores SIGHUP (and reads nothing from the terminal).
    t.tilr("new", "-d", "--", "sh", "-c", f"trap '' HUP; echo $$ > {pids}/program; exec sleep 1000",
           env=env, check=True)
    client.write(f"nohup sh -c 'echo $$ > {pids}/nohup; exec sleep 1000' >/dev/null 2>&1 &\r")
    client.write(f"sh -c 'trap \"\" HUP; echo $$ > {pids}/foreground; exec sleep 1000'\r")
    t.eventually(lambda: all(pid(n) for n in ("program", "nohup", "foreground")),
                 "the three programs run", timeout=15)
    daemon = t.tilrd_daemons()[0]
    before = [p.pid for p in descendants(daemon.pid)]
    t.ok(pid("program") in before and pid("foreground") in before and pid("nohup") in before,
         f"all three run under the server: {before}")

    t.tilr("kill-server", env=env, check=True)
    left = [n for n in before if alive(n)]
    try:
        t.ok(not alive(pid("program")), "the pane's program that ignores SIGHUP is gone")
        t.ok(not alive(pid("foreground")), "the foreground job that ignores SIGHUP is gone")
        t.ok(alive(pid("nohup")), "the nohup job is left, as a hangup leaves it")
        t.eq(left, [pid("nohup")], "nothing else of the server is left")
        client.expect(r"\[tilrd stopped\]", timeout=10)
    finally:
        if alive(pid("nohup")):
            os.kill(pid("nohup"), signal.SIGKILL)
