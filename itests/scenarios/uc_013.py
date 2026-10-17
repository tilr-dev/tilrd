"""UC-013: "SSH dropped": the client's session and terminal die (SIGHUP to the group);
tilrd and the shells in its panes live on, and a new client finds the panes as they were.

The SSH login is a terminal session: a pty, `bash --login -i` as session leader with the
pty as controlling terminal. Hanging it up closes the pty (the kernel sends SIGHUP to
bash) and sends SIGHUP to bash's process group; bash, as an interactive shell does,
passes SIGHUP on to its jobs.

Until `tilr` starts tilrd detached (UC-010), a user starts the server the way the README
says, from the login shell: `tilrd &`. tilrd ignores SIGHUP and runs each pane
process in a session of its own, so the first case checks that nothing of it goes with
the session. The second case starts it with setsid, the way scripts/update-test.sh does.
"""

import time

from tt import T, case


def prepare(t: T, start: str):
    """A login session that starts tilrd with `start`; a client with two panes in it."""
    t.need("linux", "procfs")
    dir = t.scratch("tilrd")
    env = t.server_env(dir, t.root / "state")
    log = dir / "log"
    ssh = t.session("bash", "--login", "-i", env=env, name="ssh")
    ssh.expect(r"\$ $")
    ssh.write(f"{start} >{log} 2>&1 &\r")
    server = t.found_server(log, dir, env)

    d = t.driver("before")
    d.connect(server)
    shell = d.shell_pane()
    d.line(shell, "export MARK=survived; sleep 9999 &")
    before = d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)")
    t.eq(before.split()[::2], ["survived", "1"], "the shell has its variable and its job")
    counter = d.shell_pane()
    d.line(counter, "i=0; while :; do i=$((i+1)); echo \"tick $i\"; sleep 0.2; done")
    d.wait_for(counter, r"tick 3\b")
    panes = sorted(p["pane"] for p in d.list())
    return ssh, server, d, shell, before, panes


def after_hangup(t: T, ssh, server, d, shell, before, panes):
    ssh.hangup()
    t.eventually(lambda: ssh.p.poll() is not None, "the login shell exits with its terminal", timeout=5)
    time.sleep(2)
    t.ok(server.alive(), f"tilrd (pid {server.pid}) is still running after the SSH session hung up")
    d.quit()

    again = t.driver("after")
    again.connect(server)
    t.eq(sorted(p["pane"] for p in again.list()), panes, "the same panes")
    again.cmd("subscribe", pane=shell)
    t.eq(again.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)"), before,
         "the same shell, with its variable and its background job")
    counter = next(p for p in panes if p != shell)
    again.cmd("subscribe", pane=counter)
    n = int(again.wait_for(counter, r"tick (\d+)")["groups"][1])
    again.wait_for(counter, rf"tick {n + 3}\b")
    print("  ok: the counter keeps counting")


@case(fixture="server_state")
def tilrd_started_from_an_ssh_login_shell_survives_the_session_dropping(t: T):
    """UC-013: `tilrd &` in an SSH login shell; the connection drops; panes live on."""
    after_hangup(t, *prepare(t, "tilrd --listen 127.0.0.1:0"))


@case(fixture="server_state")
def tilrd_started_with_setsid_keeps_its_panes_when_the_ssh_session_drops(t: T):
    """UC-013: tilrd detached by hand (setsid) from an SSH session; the session drops; panes live on."""
    after_hangup(t, *prepare(t, "setsid tilrd --listen 127.0.0.1:0"))
