"""UC-021: no lingering to be had (loginctl enable-linger refused), or no systemd user
manager at all: `tilr service install` writes a `@reboot` line in the user's crontab,
says that is what it chose, and starts tilrd now. The line, run as cron would at boot,
starts tilrd on the user's socket. Neither systemd nor crontab: an error, nothing
written. The user's own crontab lines stay; a second install does not add a second line.

systemctl, loginctl and crontab are scripts (tt.FakeServiceTools): the crontab is a file.
"""

import os
import socket
from pathlib import Path

from tt import FakeServiceTools, T, case, changes, snapshot

MINE = "0 3 * * * /usr/bin/backup --quiet"


def ours(fake) -> list[str]:
    return [l for l in (fake.crontab() or "").splitlines() if l.endswith("# tilr service")]


def boot(t: T, user, fake, env):
    """Stop the server and run the @reboot line as cron would: tilrd is back on the socket."""
    t.eq(user.run("tilr", "kill-server", env=env).code, 0, "tilr kill-server")
    line = ours(fake)[0]
    t.spawn("runuser", "-u", user.name, "--", "env", "-i", f"HOME={user.home}", "PATH=/usr/bin:/bin",
            "SHELL=/bin/sh", "sh", "-c", line[len("@reboot "):])
    sock = Path("/tmp") / f"tilr-{user.uid}" / "tilrd.sock"

    def up():
        try:
            with socket.socket(socket.AF_UNIX) as s:
                s.connect(str(sock))
            return True
        except OSError:
            return False
    t.eventually(up, "tilrd started by the @reboot line serves the socket", timeout=15)
    daemons = t.tilrd_daemons()
    t.eq(len(daemons), 1, "one tilrd")
    t.has(daemons[0].cmdline, f"--socket {sock}")
    t.lacks(daemons[0].cmdline, "--lock-fd")
    t.eq(user.run("tilr", "new", "-d", env=env).code, 0, "tilr uses it")
    t.eq([d.pid for d in t.tilrd_daemons()], [daemons[0].pid], "that one, no other")


@case(spec=["tilr-cli/service-install-with-cron"])
def without_lingering_a_crontab_reboot_line_is_written(t: T):
    """UC-021: enable-linger refused: a @reboot line, the choice said, no unit, tilrd started."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger="no", linger_ok=False)
    env = {"PATH": fake.path()}
    r = user.run("tilr", "service", "install", env=env)
    t.eq(r.code, 0, f"tilr service install:\n{r.text}")
    t.has(r.err, "lingering is not available", "cron @reboot")
    t.eq(len(ours(fake)), 1, "one @reboot line")
    t.has(ours(fake)[0], '"/usr/local/bin/tilrd" --socket')
    t.ok(not (user.home / ".config/systemd/user/tilr.service").exists(), "no unit")
    t.lacks("\n".join(fake.called()), "enable tilr", "daemon-reload")
    t.eq(len(t.tilrd_daemons()), 1, "tilrd started now")
    boot(t, user, fake, env)


@case(spec=["tilr-cli/service-install-with-cron"])
def without_systemd_a_crontab_reboot_line_is_written_next_to_the_users_own(t: T):
    """UC-021: no systemd user manager: the @reboot line next to the user's lines; once, however often installed."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=False)
    fake.crontab_file.write_text(MINE + "\n")
    os.chown(fake.crontab_file, user.uid, user.gid)
    env = {"PATH": fake.path()}
    r = user.run("tilr", "service", "install", env=env)
    t.eq(r.code, 0, f"tilr service install:\n{r.text}")
    t.has(r.err, "no systemd user manager", "cron @reboot")
    t.eq(user.run("tilr", "service", "install", env=env).code, 0, "installed again")
    lines = fake.crontab().splitlines()
    t.eq(lines[0], MINE, "the user's line kept")
    t.eq(len(ours(fake)), 1, "one @reboot line after two installs")
    t.eq(len(lines), 2, "and nothing else")
    t.lacks("\n".join(fake.called()), "loginctl")
    boot(t, user, fake, env)


@case(spec=["tilr-cli/service-install-with-cron"])
def with_neither_systemd_nor_crontab_install_fails_and_writes_nothing(t: T):
    """UC-021: no way to start at boot: a clear error, non-zero, nothing written or started."""
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=False, crontab=False)
    before = snapshot(user.home)
    r = user.run("tilr", "service", "install", env={"PATH": fake.path()})
    t.ok(r.code != 0, f"tilr service install fails:\n{r.text}")
    t.has(r.err, "no way to start tilrd at boot here")
    t.eq(changes(before, snapshot(user.home)), [], "nothing written in the home")
    t.eq(t.tilrd_daemons(), [], "nothing started")
