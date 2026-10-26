"""UC-003: the installer run again over an installation whose tilrd is running.

The same release again changes nothing: the installed versions are asked, nothing is
downloaded, the files stay as they are (same inode, same mtime) and the server is not
reloaded. A new release replaces both files; the installer explicitly asks the running server to
reload into it and keeps its shells: the same pid, the same panes, a
shell's variable and job. The server was started by `tilr` (no
service); a driver reaches it over its TCP port with the user's token.
"""

import os
import time
from pathlib import Path

from tt import CLIENT_NEXT_EXE, NEXT_EXE, USER_PATH, T, case


def setup(t: T):
    user = t.user()
    user.prompt()
    cdn = t.cdn()
    m = t.manifest()
    cdn.publish(m["version"])
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"}
    t.eq(user.sh(f"curl -fsSL {url} | sh", env=env).code, 0, "the first install")
    t.eq(user.run(str(bin / "tilr"), "new", "-d").code, 0, "tilr new -d")
    daemon = t.tilrd_daemons()[0]
    return user, cdn, url, env, bin, daemon


def log(user) -> str:
    return (Path("/tmp") / f"tilr-{user.uid}" / "tilrd.log").read_text()


@case(spec=["installation/installing-the-same-release-again-changes-nothing"])
def installing_the_same_release_again_changes_nothing(t: T):
    """UC-003: the same release again: files untouched, the server not reloaded, exit 0."""
    user, cdn, url, env, bin, daemon = setup(t)
    files = {n: os.stat(bin / n) for n in ("tilrd", "tilr")}
    panes = user.run(str(bin / "tilr"), "ls", "-q").out
    fetched = len(cdn.requests)

    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.eq(r.code, 0, f"the second install:\n{r.text}")
    version = t.manifest()["version"]
    t.has(r.err, f"tilr {version} is already installed")
    t.eq([p for p in cdn.requests[fetched:] if ".tar.gz" in p], [], "nothing downloaded")
    for name, st in files.items():
        now = os.stat(bin / name)
        t.eq((now.st_ino, now.st_mtime_ns), (st.st_ino, st.st_mtime_ns), f"{name} is the same file, untouched")
    time.sleep(2)
    t.eq([d.pid for d in t.tilrd_daemons()], [daemon.pid], "the same server")
    t.lacks(log(user), "reloading")
    t.eq(user.run(str(bin / "tilr"), "ls", "-q").out, panes, "the same panes")


@case(spec=[
    "installation/a-new-release-reloads-the-running-server",
    "reload/installation-requests-reload-explicitly",
    "reload/the-tcp-listener-carries-over",
])
def installing_a_new_release_reloads_the_running_server_and_keeps_its_shells(t: T):
    """UC-003: a new release over a running tilrd: it reloads in place, shells and their state kept."""
    user, cdn, url, env, bin, daemon = setup(t)
    m = t.manifest()
    token = (user.home / ".local/state/tilr/token").read_text().strip()
    d = t.driver()
    d.connect("ws://127.0.0.1:7890", token=token)
    pane = d.shell_pane()
    d.line(pane, "export KEPT=yes; sleep 9999 &")
    shell = int(d.value(pane, "$$"))
    panes = user.run(str(bin / "tilr"), "ls", "-q").out

    cdn.publish(m["next_version"], server=NEXT_EXE, client=CLIENT_NEXT_EXE)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.eq(r.code, 0, f"the update:\n{r.text}")
    t.has(r.err, f"downloading tilr {m['next_version']}", "the running tilrd reloads itself and keeps its shells")
    for name in ("tilrd", "tilr"):
        t.has(user.run(str(bin / name), "--version").out, m["next_version"])
    t.eventually(lambda: "resumed after reload" in log(user), "the server reloads into the new release", timeout=30)
    t.has(user.run(str(bin / "tilr"), "version").out, f"tilrd {m['next_version']}")
    t.eq([x.pid for x in t.tilrd_daemons()], [daemon.pid], "the same server process")
    t.eq(user.run(str(bin / "tilr"), "ls", "-q").out, panes, "the same panes")
    d.cmd("reconnect", timeout_ms=20000)
    t.eq(d.value(pane, "$KEPT-$$-$(jobs -r | grep -c sleep)"), f"yes-{shell}-1", "the same shell, its variable and job")
