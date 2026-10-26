"""UC-013: "SSH dropped": the client's session and terminal die (SIGHUP to the group);
tilrd and the shells in its panes live on, and a new client finds the panes as they were.

The SSH login is a terminal session: a pty, `bash --login -i` as session leader with the
pty as controlling terminal. Hanging it up closes the pty (the kernel sends SIGHUP to
bash) and sends SIGHUP to bash's process group; bash, as an interactive shell does,
passes SIGHUP on to its jobs.

The first case is the tmux model: `tilr` typed in the SSH session starts tilrd (detached,
UC-010) and attaches; the session drops, taking `tilr` with it; a `tilr` in a new
session finds the same shell. The others start the server by hand from the login shell,
as a service script might: `tilrd &` (tilrd ignores SIGHUP and runs each pane process in
a session of its own, so nothing of it goes with the session), and with setsid, the way
scripts/update-test.sh does.
"""

import time

from tt import T, case


@case(spec=["daemon-lifecycle/sighup-is-ignored-and-panes-have-their-own-sessions"])
def tilr_in_an_ssh_session_leaves_tilrd_and_its_shell_when_the_session_drops(t: T):
    """UC-013: `tilr` in an SSH login shell; the connection drops; a new tilr finds the same shell."""
    t.need("linux", "procfs")
    env = t.user_env()
    ssh = t.session("bash", "--login", "-i", env=env, name="ssh")
    ssh.expect(r"\$ $")
    ssh.write("tilr\r")
    t.eventually(t.tilrd_daemons, "tilr started a tilrd", timeout=15)
    ssh.write('export MARK=survived; sleep 9999 & echo "shell-$$-ok"\r')
    shell = int(ssh.expect(r"shell-(\d+)-ok").group(1))
    daemon = t.tilrd_daemons()[0]

    ssh.hangup()
    t.eventually(lambda: ssh.p.poll() is not None, "the login shell exits with its terminal", timeout=5)
    t.eventually(lambda: not t.procs(r"(^|/)tilr( |$)"), "tilr goes with the session", timeout=5)
    time.sleep(2)
    t.ok(t.proc(daemon.pid) and t.proc(daemon.pid).alive, "tilrd is still running")
    t.ok(t.proc(shell) and t.proc(shell).alive, f"the pane's shell (pid {shell}) is still running")

    again = t.session("tilr", env=env, name="again")
    again.prompt(timeout=15)
    again.write('echo "back-$MARK-$$-$(jobs -r | grep -c sleep)"\r')
    m = again.expect(r"back-(\w+)-(\d+)-(\d)")
    t.eq((m.group(1), int(m.group(2)), m.group(3)), ("survived", shell, "1"),
         "the same shell, with its variable and its job")
    t.eq([p.pid for p in t.tilrd_daemons()], [daemon.pid], "the same tilrd, no other")


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


@case(fixture="server_state", spec=["daemon-lifecycle/sighup-is-ignored-and-panes-have-their-own-sessions"])
def tilrd_started_from_an_ssh_login_shell_survives_the_session_dropping(t: T):
    """UC-013: `tilrd &` in an SSH login shell; the connection drops; panes live on."""
    after_hangup(t, *prepare(t, "tilrd --listen 127.0.0.1:0"))


@case(fixture="server_state", spec=["daemon-lifecycle/sighup-is-ignored-and-panes-have-their-own-sessions"])
def tilrd_started_with_setsid_keeps_its_panes_when_the_ssh_session_drops(t: T):
    """UC-013: tilrd detached by hand (setsid) from an SSH session; the session drops; panes live on."""
    after_hangup(t, *prepare(t, "setsid tilrd --listen 127.0.0.1:0"))
