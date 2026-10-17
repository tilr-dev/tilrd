"""UC-051: a 3G-like network (netem): interactive work goes on, input latency is measured.

Layer 2: the client in a container of its own ("phone"); netem on both ends shapes only
the server port's packets between it and tilrd: 150 ms ± 20 ms each way (an RTT of about
300 ms), 1% loss each way (about 2% per round trip) and 10% of packets reordered. Input
latency is what the client measures: from sending an input to the first frame that
carries its seq (Frame.last_input_seq), on the client's clock.
"""

import re

from tt import T, case

NETEM = "delay 150ms 20ms loss 1% reorder 10% 50%"
COMMANDS = 30


@case(fixture="server_state", hosts=["phone"], net_admin=True)
def a_shell_stays_usable_on_a_3g_network_and_input_latency_is_recorded(t: T):
    """UC-051: 3G profile: typing keys and running commands works, each output once; latency percentiles recorded."""
    server = t.server(listen="0.0.0.0:0", state=t.root / "state")
    phone_host = t.host("phone")
    me = phone_host.peer_ip
    phone = phone_host.driver("phone")
    phone.connect(f"ws://{me}:{server.port}", token=server.token())
    phone.cmd("keepalive", interval_ms=2000, timeout_ms=15_000, auto_reconnect=True)
    pane = phone.shell_pane(100, 30)
    phone_host.net.netem(NETEM, dst=me, port=server.port)
    t.net().netem(NETEM, dst=phone_host.ip, port=server.port)
    since = phone.mark()
    rtt = [phone.cmd("ping")["rtt_ms"] for _ in range(5)]
    t.metric("ping_rtt_ms_median", sorted(rtt)[2])

    # Keys one by one, as a person types: every key its own input message.
    phone.latency(reset=True)
    for ch in "echo typed-by-keys":
        phone.input(pane, ch)
        phone.cmd("sleep", ms=80, quiet=True)
    phone.key(pane, "enter")
    phone.wait_for(pane, r"(?m)^typed-by-keys$", timeout=20)
    keys = phone.latency(reset=True)
    for k in ("count", "p50_ms", "p90_ms", "max_ms"):
        t.metric(f"key_latency_{k}", keys[k])

    # Whole commands, each waited for.
    for n in range(1, COMMANDS + 1):
        t.eq(phone.run(pane, f"echo out-{n}", timeout=20), 0, f"command {n}")
    lines = phone.latency()
    for k in ("count", "p50_ms", "p90_ms", "p99_ms", "max_ms", "unanswered"):
        t.metric(f"input_latency_{k}", lines[k])

    t.ok(not any(e.get("ev") == "disconnected" for e in phone.events[since:]), "the session never broke")
    t.ok(keys["count"] >= len("echo typed-by-keys"), "every key was echoed")
    t.ok(keys["p90_ms"] is not None and keys["p90_ms"] < 2000, f"key echo p90 under 2 s ({keys['p90_ms']} ms)")
    t.ok(lines["p99_ms"] is not None and lines["p99_ms"] < 5000, f"input latency p99 under 5 s ({lines['p99_ms']} ms)")
    text = phone.text(pane)
    t.eq(len(re.findall(rf"(?m)^out-{COMMANDS}$", text)), 1, "the last command's output appears once")
