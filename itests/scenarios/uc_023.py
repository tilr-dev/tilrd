"""UC-023: without `tilr service install` nothing starts tilrd at boot or at login: after
installing (no systemd here) and using tilr every way a user does, there is no unit, no
crontab line, no autostart entry, no edited profile, and tilr asked no service manager
anything. Explicitness, not a rootkit.

systemctl, loginctl and crontab are scripts that note being called (tt.FakeServiceTools,
no systemd user manager: the installer's own probe answers "no").
"""

from pathlib import Path

from tt import FakeServiceTools, T, case, changes, snapshot


@case()
def nothing_starts_tilrd_at_boot_unless_asked(t: T):
    """UC-023: install and use tilr: no unit, crontab, autostart or profile edit anywhere."""
    user = t.user()
    user.prompt()
    fake = FakeServiceTools(t, owner=user, systemd=False)
    cdn = t.cdn()
    cdn.publish(t.manifest()["version"])
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{fake.path()}"}
    system = {d: snapshot(Path(d)) for d in ("/etc", "/var/spool")}
    home = snapshot(user.home)

    t.eq(user.sh(f"curl -fsSL {url} | sh", env=env).code, 0, "the install")
    installer_calls = fake.called()
    t.eq([c for c in installer_calls if c != "systemctl --user show-environment"], [],
         "the installer only asked whether there is systemd")
    for args in (("new", "-d"), ("ls",), ("token",), ("version",), ("ps",), ("kill-server",), ("new", "-d"),
                 ("kill",), ("new", "-d")):
        t.eq(user.run("tilr", *args, env=env).code, 0, f"tilr {' '.join(args)}")
    t.eq(fake.called()[len(installer_calls):], [], "tilr asked no service manager anything")
    t.eq(fake.crontab(), None, "no crontab")
    added = changes(home, snapshot(user.home, skip=(".local/state", ".local/bin")))
    t.eq([a for a in added if not a.startswith("+ .local:")], [], "nothing else in the home")
    for d, before in system.items():
        t.eq(changes(before, snapshot(Path(d))), [], f"nothing changed in {d}")
    for path in (".config/systemd", ".config/autostart", "Library/LaunchAgents"):
        t.ok(not (user.home / path).exists(), f"no ~/{path}")
