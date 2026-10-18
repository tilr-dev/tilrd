"""UC-005: a clean system: no $XDG_RUNTIME_DIR, no ~/.config, nothing of tilr's yet. After
the install the first `tilr` makes what it needs, private: the runtime directory
(/tmp/tilr-<uid>) and what is under it 0700 / 0600 (sockets, lock, log; the panes'
helper links are links), the state
directory (~/.local/state/tilr) 0700 with the token 0600.
"""

import os
import stat
from pathlib import Path

from tt import USER_PATH, T, case


def private(t: T, root: Path, uid: int):
    """root and everything under it: the user's; directories 0700, sockets 0600, other
    files nothing for group and others (links: only whose they are)."""
    found = [root] + sorted(root.rglob("*"))
    for path in found:
        st = path.lstat()
        mode = stat.S_IMODE(st.st_mode)
        what = f"{path} ({stat.filemode(st.st_mode)})"
        t.eq(st.st_uid, uid, f"{what} is the user's")
        if stat.S_ISLNK(st.st_mode):
            continue
        if stat.S_ISDIR(st.st_mode):
            t.eq(oct(mode), oct(0o700), what)
        elif stat.S_ISSOCK(st.st_mode):
            t.eq(oct(mode), oct(0o600), what)
        else:
            t.eq(oct(mode & 0o077), oct(0), f"{what}: nothing for group and others")
    return found


@case(spec=[
    "daemon-lifecycle/runtime-directory-and-socket-modes",
    "daemon-lifecycle/state-directory",
    "daemon-lifecycle/unix-socket-location",
    "installation/the-first-run-makes-private-directories",
])
def on_a_clean_system_tilr_makes_its_directories_private(t: T):
    """UC-005: no XDG_RUNTIME_DIR, no ~/.config: paths made, directories 0700, files and sockets 0600."""
    user = t.user()
    t.ok(not (user.home / ".config").exists(), "no ~/.config before")
    cdn = t.cdn()
    cdn.publish(t.manifest()["version"])
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"}
    t.eq(user.sh(f"curl -fsSL {url} | sh", env=env).code, 0, "the install")

    t.eq(user.run(str(bin / "tilr"), "new", "-d", env=env).code, 0, "tilr new -d")
    token = user.run(str(bin / "tilr"), "token", env=env)
    t.eq(token.code, 0, "tilr token")
    runtime = Path("/tmp") / f"tilr-{user.uid}"
    found = private(t, runtime, user.uid)
    names = {p.name for p in found}
    t.ok({"tilrd.sock", "tilrd.sock.lock", "tilrd.log"} <= names, f"socket, lock and log in {runtime}: {names}")
    t.ok(any(p.parent.name == "agent" for p in found), "the pane's agent socket is there too")
    state = user.home / ".local/state/tilr"
    private(t, state, user.uid)
    t.eq((state / "token").read_text().strip(), token.out.strip(), "tilr token prints the token kept there")
    config = user.home / ".config"
    if config.exists():
        for path in [config] + sorted(config.rglob("*")):
            t.eq(path.lstat().st_uid, user.uid, f"{path} is the user's")
            t.ok(not path.lstat().st_mode & 0o022, f"{path} is not writable by others")
    print(f"  ~/.config after: {sorted(str(p) for p in config.rglob('*')) if config.exists() else 'none'}")
    user.run(str(bin / "tilr"), "kill-server", env=env)
    t.ok(not os.path.exists(runtime / "tilrd.sock"), "the socket goes with the server")
