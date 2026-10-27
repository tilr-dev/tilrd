"""UC-023: without `tilr service install` nothing starts tilrd at boot or at login: after
installing and using tilr every way a user does, there is no unit, no crontab line, no
autostart entry, no lingering, no edited profile, and neither the installer nor tilr
asked a service manager anything, though a systemd user manager is there and would
answer. Explicitness, not a rootkit.

systemctl, loginctl and crontab are scripts that note being called (tt.FakeServiceTools,
a systemd user manager, lingering off).
"""

from pathlib import Path

from tt import FakeServiceTools, T, case, changes, snapshot


@case(spec=["installation/the-installer-sets-up-no-service", "tilr-cli/nothing-starts-tilrd-at-boot-unless-asked"])
def nothing_starts_tilrd_at_boot_unless_asked(t: T):
    """UC-023: install (systemd there) and use tilr: no unit, crontab, linger, autostart or profile edit."""
    user = t.user()
    user.prompt()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger="no")
    cdn = t.cdn()
    cdn.publish(t.manifest()["version"])
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{fake.path()}"}
    system = {d: snapshot(Path(d)) for d in ("/etc", "/var/spool")}
    home = snapshot(user.home)

    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.eq(r.code, 0, f"the install:\n{r.text}")
    t.has(r.err, "start tilrd at boot: tilr service install")
    installer_calls = fake.called()
    t.eq(installer_calls, [], "the installer asked systemctl, loginctl and crontab nothing")
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
