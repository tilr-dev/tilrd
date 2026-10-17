"""UC-011: a second `tilr` connects to the tilrd the first one started; no second daemon
appears. Both show the same pane (the one used last), and what one types the other sees.
"""

from tt import T, case


@case()
def a_second_tilr_attaches_to_the_same_daemon_and_pane(t: T):
    """UC-011: the second tilr finds the first one's tilrd and its pane; still one daemon."""
    t.need("procfs")
    env = t.user_env()
    first = t.session("tilr", env=env, name="first")
    first.prompt(timeout=15)
    daemons = t.tilrd_daemons()
    t.eq(len(daemons), 1, "one tilrd after the first tilr")

    second = t.session("tilr", env=env, name="second")
    second.prompt(timeout=15)
    t.eq([p.pid for p in t.tilrd_daemons()], [daemons[0].pid], "the same tilrd, no other")
    t.eq(len(t.tilr("ls", "-q", env=env, check=True).out.split()), 1, "one pane, shown in both")

    second.write('echo "from-$((6*7))-second"\r')
    first.expect(r"from-42-second", timeout=10)
    print("  ok: what the second types shows in the first")
