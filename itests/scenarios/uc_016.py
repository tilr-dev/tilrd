"""UC-016: tilrd run by hand in the foreground (as a service manager runs it): it logs to
stderr, serves its Unix socket and the TCP port 127.0.0.1:7890 (the Tilr app and
tunnels), and `tilr` attaches to it instead of starting another.
"""

import signal

from tt import SERVER_EXE, T, case


@case(spec=["daemon-lifecycle/foreground-service-with-its-log-on-stderr", "tilr-cli/how-an-attach-ends"])
def tilr_attaches_to_a_tilrd_running_in_the_foreground(t: T):
    """UC-016: `tilrd` in the foreground, log on stderr; tilr uses it; SIGTERM stops it cleanly."""
    t.need("procfs")
    env = t.user_env()
    log = t.scratch("fg") / "stderr"
    server = t.spawn(SERVER_EXE, env=env, log=log)
    t.eventually(lambda: "Unix socket for local clients" in log.read_text(), "tilrd says where it listens")
    t.has(log.read_text(), "listening on ws://127.0.0.1:7890")

    client = t.session("tilr", env=env, name="tilr")
    client.prompt(timeout=15)
    t.eq([p.pid for p in t.tilrd_daemons()], [server.pid], "the foreground tilrd, no other")
    t.has(log.read_text(), "client connected", "addr=unix")
    client.write('echo "fg-$((40+2))"\r')
    client.expect("fg-42")

    server.send_signal(signal.SIGTERM)
    t.eq(server.wait(10), 0, "tilrd exits 0 on SIGTERM")
    client.expect(r"\[tilrd stopped\]", timeout=10)
    t.eq(client.wait_exit(), 0, "the attached tilr ends")
