"""UC-068: a reload at the moment a pane is being created. The supervisor puts a reload off
while a pane is starting (it has not reported yet) and tries again a moment later, so a
pane created around a reload either is there afterwards, whole, or was never made: never
a pane process the server does not list, nor a listed pane without its process, nor a
reload that is lost.

Twenty rounds: CreatePane goes out and SIGUSR1 follows after 0 to 40 ms (a thread, so the
two race); then the client is back. After every round each pane the server lists has
its pane process and answers in its shell, every pane process is a listed pane, and the
reload happened.
"""

import random
import signal
import threading

from tt import T, case
from tt.live import state_files, wait_reloads

ROUNDS = 20


@case(fixture="server_state", timeout=420)
def panes_created_while_the_server_reloads_are_whole_or_absent_and_every_reload_happens(t: T):
    """UC-068: 20 rounds of CreatePane racing SIGUSR1: listed panes and pane processes match, all answer, all reloads done."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    d.cmd("keepalive", interval_ms=500, timeout_ms=3000, auto_reconnect=True, reconnect_timeout_ms=30_000)
    d.shell_pane()
    rng = random.Random(68)
    created, missing_answers, delays = 0, 0, []
    for n in range(1, ROUNDS + 1):
        delay = rng.uniform(0, 0.04)
        delays.append(round(delay * 1000))
        since = d.mark()
        timer = threading.Timer(delay, server.signal, args=(signal.SIGUSR1,))
        timer.start()
        answer = d.cmd("create_pane", cols=80, rows=24, ok=False, timeout_ms=10_000)
        timer.join()
        if answer.get("ev") == "pane_created":
            created += 1
        else:
            missing_answers += 1
            print(f"  round {n}: no PaneCreated ({answer.get('message')})")
        wait_reloads(t, server, n, timeout=20)
        if not d.connected() or any(e.get("ev") == "disconnected" for e in d.events[since:]):
            t.eventually(lambda: d.connected(), "the client is back", timeout=20)

        listed = {p["pane"]: p["pid"] for p in d.list()}
        processes = {p.pid for p in server.panes()}
        t.eq(sorted(listed.values()), sorted(processes),
             f"round {n}: the panes listed and the pane processes running are the same")
    t.metric("rounds", ROUNDS)
    t.metric("pane_created_answers", created)
    t.metric("no_answer", missing_answers)
    t.metric("signal_delays_ms", delays)
    t.eq(server.logged("resumed after reload with"), ROUNDS, "every reload happened, once")

    panes = [p["pane"] for p in d.list()]
    t.ok(len(panes) >= 1 + created, f"every pane whose creation was answered is there ({len(panes)} panes)")

    def every_pane_answers():
        dead = []
        for pane in panes:
            d.cmd("subscribe", pane=pane)
            d.wait_for(pane, r"(?m)^\$ ?$", timeout=15)
            if d.run(pane, "echo alive") != 0:
                dead.append(pane)
        t.eq(dead, [], "panes whose shell does not answer")
    t.check("every pane there answers in its shell", every_pane_answers)
    t.check("no reload state is left behind", lambda: t.eq(state_files(server), [], "state files"))
    t.check("nothing was lost in the log", lambda: t.eq(
        [l for l in server.log_text().splitlines() if "pane lost" in l or "panicked" in l or "not reloading" in l],
        [], "bad log lines"))
