"""UC-03A: `tilr` says nothing to a socket another user serves. Another user (mallory) can
make the victim's default runtime directory in /tmp before the victim does (no
XDG_RUNTIME_DIR: /tmp/tilr-<uid>) and serve a socket there, or serve one at a path the
victim names (TILR_SOCKET, --socket, client.toml's `socket`). `tilr` refuses: a runtime
directory that is not the user's alone is not entered (nothing connected, no tilrd
started); a socket whose peer runs as another uid is closed before the WebSocket handshake
(not a byte sent).
"""

from pathlib import Path

from tt import CLIENT_EXE, T, case

# The other user's socket: accepts, notes how many bytes each connection sent before it
# ended (one line per connection) in REPORT.
FAKE = r'''
import os, socket, sys
path, report = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX)
s.bind(path)
os.chmod(path, 0o666)
s.listen(16)
open(report + ".ready", "w").close()
while True:
    c, _ = s.accept()
    c.settimeout(5)
    got = 0
    try:
        while True:
            b = c.recv(65536)
            if not b:
                break
            got += len(b)
    except OSError:
        pass
    c.close()
    with open(report, "a") as f:
        f.write(f"{got}\n")
'''


def serve_fake(t: T, owner, socket: Path) -> Path:
    """A listener of `owner` at `socket` (0666); the path of its report."""
    script = owner.home / "fake.py"
    script.write_text(FAKE)
    report = owner.home / "report"
    t.run("chown", owner.name, script, check=True, quiet=True)
    t.spawn("runuser", "-u", owner.name, "--", "python3", script, socket, report)
    t.eventually(lambda: Path(f"{report}.ready").exists(), "the fake socket listens")
    return report


def connections(report: Path) -> list[int]:
    return [int(n) for n in report.read_text().split()] if report.exists() else []


@case(spec=[
    "tilr-cli/tilr-checks-who-serves-the-socket#another-user-s-runtime-directory",
])
def a_runtime_directory_another_user_made_in_tmp_is_not_entered(t: T):
    """UC-03A: /tmp/tilr-<uid> made by another user, a socket served in it: tilr ls / attach / new refuse, nothing connects."""
    t.need("root", "procfs")
    alice, mallory = t.user("alice"), t.user("mallory")
    run = Path("/tmp") / f"tilr-{alice.uid}"
    t.eq(mallory.sh(f"mkdir -m 755 {run}").code, 0, "mallory makes alice's runtime directory first")
    report = serve_fake(t, mallory, run / "tilrd.sock")

    for args in (["ls"], ["attach"], [], ["new", "-d"]):
        r = alice.run(CLIENT_EXE, *args)
        t.ok(r.code != 0, f"tilr {' '.join(args)} refuses: {r.err}")
        t.has(r.err, str(run), f"belongs to another user (uid {mallory.uid})")
    t.eq(connections(report), [], "nothing connected to the other user's socket")
    t.eq(t.tilrd_daemons(), [], "no tilrd was started")
    t.ok(not (run / "tilrd.sock.lock").exists() and not (run / "tilrd.log").exists(),
         "nothing of alice's was made in mallory's directory")


@case(spec=[
    "tilr-cli/tilr-checks-who-serves-the-socket#a-socket-served-by-another-user",
])
def a_socket_another_user_serves_at_a_named_path_is_refused_before_a_byte(t: T):
    """UC-03A: a socket served by another uid at TILR_SOCKET / --socket / client.toml: refused, 0 bytes sent."""
    t.need("root", "procfs")
    alice, mallory = t.user("alice"), t.user("mallory")
    where = Path("/tmp/elsewhere")
    t.eq(mallory.sh(f"mkdir -m 755 {where}").code, 0, "mallory's directory")
    socket = where / "tilrd.sock"
    report = serve_fake(t, mallory, socket)
    served = f"socket {socket} is served by another user (uid {mallory.uid})"

    r = alice.run(CLIENT_EXE, "ls", env={"TILR_SOCKET": socket})
    t.ok(r.code != 0, f"TILR_SOCKET: refused: {r.err}")
    t.has(r.err, served)
    t.eventually(lambda: len(connections(report)) == 1, lambda: f"one connection: {connections(report)}")

    r = alice.run(CLIENT_EXE, "--socket", socket, "new", "-d")
    t.ok(r.code != 0, f"--socket: refused: {r.err}")
    t.has(r.err, served)

    config = alice.home / ".config/tilr"
    t.eq(alice.sh(f"mkdir -p {config} && echo 'socket = \"{socket}\"' >{config}/client.toml").code, 0, "client.toml")
    r = alice.run(CLIENT_EXE, "attach")
    t.ok(r.code != 0, f"client.toml: refused: {r.err}")
    t.has(r.err, served)

    t.eventually(lambda: len(connections(report)) >= 3, lambda: f"each tilr connected: {connections(report)}")
    t.eq(set(connections(report)), {0}, "not a byte was sent to the other user's socket")
    t.eq(t.tilrd_daemons(), [], "no tilrd was started")
