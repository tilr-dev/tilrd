"""UC-006: not root, no sudo, no systemd (a container): the installer installs and says
what to type next, and `tilr` then does everything: starts tilrd, panes, the token, the
TCP port for the app. `sudo` is a script that notes being called: it never is.
"""

from tt import USER_PATH, T, case


@case(spec=["installation/without-systemd-the-install-still-works"])
def without_root_sudo_or_systemd_everything_works(t: T):
    """UC-006: an unprivileged user in a container without systemd installs and uses tilr."""
    user = t.user()
    user.prompt()
    trap = t.scratch("trap")
    (trap / "sudo").write_text(f"#!/bin/sh\necho \"$@\" >>{trap}/sudo-called\nexit 1\n")
    (trap / "sudo").chmod(0o755)
    trap.chmod(0o777)
    t.eq(t.sh("command -v systemctl").code, 1, "no systemctl in this container")
    cdn = t.cdn()
    version = t.manifest()["version"]
    cdn.publish(version)
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{trap}:{USER_PATH}"}

    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.eq(r.code, 0, f"the install:\n{r.text}")
    t.has(r.err, "open a shell: tilr", "start tilrd at boot: tilr service install")
    t.lacks(r.err, "systemd", "service was set up")
    t.eq(user.run("tilr", "new", "-d", env=env).code, 0, "tilr new -d")
    t.eq(len(user.run("tilr", "ls", "-q", env=env).out.split()), 1, "one pane")
    token = user.run("tilr", "token", env=env)
    t.eq(token.code, 0, "tilr token")
    d = t.driver()
    d.connect("ws://127.0.0.1:7890", token=token.out.strip())
    pane = d.shell_pane()
    t.eq(d.value(pane, "$(id -un)"), user.name, "the pane's shell is the user's")
    t.has(user.run("tilr", "version", env=env).out, f"tilrd {version}")
    t.ok(not (trap / "sudo-called").exists(), "sudo was never called")

