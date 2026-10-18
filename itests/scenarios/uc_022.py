"""UC-022: `tilr service uninstall` removes what `tilr service install` added, and only
that: its unit (disabled first, lingering turned off if install turned it on), its
@reboot line (the crontab itself if install made it), nothing else of the user's. It
stops nothing: the running tilrd and its panes go on. A unit it did not write (a
tilr.service the user wrote by hand) is left alone.

systemctl, loginctl and crontab are scripts (tt.FakeServiceTools).
"""

import os

from tt import FakeServiceTools, T, case, changes, snapshot

MINE = "0 3 * * * /usr/bin/backup --quiet"
HAND_WRITTEN_UNIT = """[Unit]
Description=tilr terminal server

[Service]
ExecStart=/bin/sh -lc 'exec "/home/alice/.local/bin/tilrd"'
Restart=on-failure

[Install]
WantedBy=default.target
"""


@case()
def uninstall_removes_the_unit_it_wrote_and_nothing_else(t: T):
    """UC-022: the unit disabled and removed, lingering off again, other units and the server untouched."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger="no")
    env = {"PATH": fake.path()}
    units = user.home / ".config/systemd/user"
    units.mkdir(parents=True)
    (units / "other.service").write_text("[Service]\nExecStart=/bin/true\n")
    t.sh(f"chown -R {user.name}: {user.home}/.config", check=True)
    t.eq(user.run("tilr", "new", "-d", env=env).code, 0, "tilr new -d")
    daemon = t.tilrd_daemons()[0]
    panes = user.run("tilr", "ls", "-q", env=env).out
    before = snapshot(user.home, skip=(".local/state",))

    t.eq(user.run("tilr", "service", "install", env=env).code, 0, "install")
    t.ok((units / "tilr.service").exists(), "the unit is there")
    mark = len(fake.called())
    r = user.run("tilr", "service", "uninstall", env=env)
    t.eq(r.code, 0, f"tilr service uninstall:\n{r.text}")
    t.eq(changes(before, snapshot(user.home, skip=(".local/state",))), [], "the home as before install")
    calls = fake.called()[mark:]
    t.ok("systemctl --user disable tilr.service" in calls, f"disabled: {calls}")
    t.ok(f"loginctl disable-linger {user.name}" in calls, "lingering off again")
    t.lacks("\n".join(calls), "stop", "--now")
    t.eq([d.pid for d in t.tilrd_daemons()], [daemon.pid], "the server goes on")
    t.eq(user.run("tilr", "ls", "-q", env=env).out, panes, "with its panes")


@case()
def uninstall_leaves_lingering_it_did_not_turn_on(t: T):
    """UC-022: lingering was on before install: uninstall leaves it on."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger="yes")
    env = {"PATH": fake.path()}
    t.eq(user.run("tilr", "service", "install", env=env).code, 0, "install")
    t.eq(user.run("tilr", "service", "uninstall", env=env).code, 0, "uninstall")
    t.lacks("\n".join(fake.called()), "disable-linger")


@case()
def uninstall_removes_its_crontab_line_and_keeps_the_users(t: T):
    """UC-022: the @reboot line goes, the user's lines stay as they were."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=False)
    fake.crontab_file.write_text(MINE + "\n")
    os.chown(fake.crontab_file, user.uid, user.gid)
    env = {"PATH": fake.path()}
    t.eq(user.run("tilr", "service", "install", env=env).code, 0, "install")
    t.ok("# tilr service" in fake.crontab(), "the @reboot line is there")
    r = user.run("tilr", "service", "uninstall", env=env)
    t.eq(r.code, 0, f"tilr service uninstall:\n{r.text}")
    t.eq(fake.crontab(), MINE + "\n", "the crontab as it was")


@case()
def uninstall_removes_the_crontab_it_made(t: T):
    """UC-022: no crontab before install: none after uninstall."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=False)
    env = {"PATH": fake.path()}
    t.eq(user.run("tilr", "service", "install", env=env).code, 0, "install")
    t.eq(user.run("tilr", "service", "uninstall", env=env).code, 0, "uninstall")
    t.eq(fake.crontab(), None, "no crontab")
    t.ok("crontab -r" in fake.called(), "removed with crontab -r")


@case()
def uninstall_leaves_what_it_did_not_write(t: T):
    """UC-022: nothing installed: nothing removed; a hand-written unit is left alone."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger="yes")
    env = {"PATH": fake.path()}
    unit = user.home / ".config/systemd/user/tilr.service"
    unit.parent.mkdir(parents=True)
    unit.write_text(HAND_WRITTEN_UNIT)
    t.sh(f"chown -R {user.name}: {user.home}/.config", check=True)
    before = snapshot(user.home)
    r = user.run("tilr", "service", "uninstall", env=env)
    t.eq(r.code, 0, f"tilr service uninstall:\n{r.text}")
    t.has(r.err, "not written by `tilr service install`: left as it is", "nothing to remove")
    t.eq(changes(before, snapshot(user.home)), [], "the home as it was")
    t.eq([c for c in fake.called() if not c.startswith("crontab -l")], [], "nothing told to systemd or loginctl")
