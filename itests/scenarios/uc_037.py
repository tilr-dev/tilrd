"""UC-037: a socket path longer than sun_path (108 bytes on Linux) is a clear error that
says how to get a shorter one; nothing is started. With TILR_SOCKET set to a short path
the same user is served.
"""

from pathlib import Path

from tt import SERVER_EXE, T, case


@case()
def a_socket_path_longer_than_sun_path_is_a_clear_error(t: T):
    """UC-037: XDG_RUNTIME_DIR so deep the socket path is over 107 bytes: tilr and tilrd say so."""
    t.need("procfs")
    env = t.user_env()
    deep = Path(env["XDG_RUNTIME_DIR"]) / ("d" * 60) / ("e" * 60)
    deep.mkdir(parents=True, mode=0o700)
    env["XDG_RUNTIME_DIR"] = str(deep)
    path = deep / "tilr" / "tilrd.sock"
    t.ok(len(str(path)) > 107, f"{len(str(path))} bytes")

    for argv in (["tilr", "new", "-d"], [SERVER_EXE]):
        r = t.run(*argv, env=env, timeout=30)
        t.ok(r.code != 0, f"{argv[0]} fails")
        t.has(r.err, f"is {len(str(path))} bytes, longer than a Unix socket's 107", "TILR_SOCKET")
    t.eq(t.tilrd_daemons(), [], "nothing was started")

    short = t.scratch("s") / "t.sock"
    out = t.tilr("new", "-d", env={**env, "TILR_SOCKET": str(short)}, check=True).out
    t.ok(out.strip(), "served at TILR_SOCKET")
    t.tilr("kill-server", env={**env, "TILR_SOCKET": str(short)}, check=True)
