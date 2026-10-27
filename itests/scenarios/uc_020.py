"""UC-020: `tilr service install` with a systemd user manager and lingering: a user unit
`~/.config/systemd/user/tilr.service` is written (ExecStart runs tilrd from a login
shell, Type=notify, Restart=on-failure, WantedBy=default.target), systemd is told to
reload and to enable it (and start it, unless a tilrd runs already).

There is no systemd in the container: systemctl and loginctl are scripts that note their
arguments (tt.FakeServiceTools); the unit is checked as a file. That Type=notify holds is
checked by running its ExecStart as systemd would, with $NOTIFY_SOCKET: tilrd must say
READY=1 there once it listens.

A tilr.service the user wrote (without the `tilr service` mark) is not replaced: install
fails naming it and tells systemd nothing.
"""

import socket
import time

from tt import FakeServiceTools, T, case


def install(t: T, linger: str = "yes", linger_ok: bool = True):
    user = t.user()
    fake = FakeServiceTools(t, owner=user, systemd=True, linger=linger, linger_ok=linger_ok)
    env = {"PATH": fake.path()}
    return user, fake, env


@case(spec=["daemon-lifecycle/ready-notification-for-a-service-manager", "tilr-cli/service-install-with-systemd",
            "panes/user-variables-of-a-pane"])
def service_install_writes_a_notify_user_unit_and_enables_it(t: T):
    """UC-020: the unit's ExecStart, Type=notify and Restart; daemon-reload and enable --now; its panes lack NOTIFY_SOCKET."""
    user, fake, env = install(t)
    r = user.run("tilr", "service", "install", env=env)
    t.eq(r.code, 0, f"tilr service install:\n{r.text}")
    unit = user.home / ".config/systemd/user/tilr.service"
    text = unit.read_text()
    print(text)
    lines = text.splitlines()
    for line in ("Type=notify", "ExecStart=/bin/sh -lc 'exec \"/usr/local/bin/tilrd\"'", "Restart=on-failure",
                 "WantedBy=default.target"):
        t.ok(line in lines, f"the unit has {line}")
    t.eq(fake.called(), ["systemctl --user show-environment",
                         f"loginctl show-user {user.name} --property=Linger --value",
                         "systemctl --user daemon-reload", "systemctl --user enable --now tilr.service"],
         "what systemd was told")
    t.has(r.err, "systemd user service", "lingering on")
    t.eq(fake.crontab(), None, "no crontab")

    # Type=notify: started as systemd would start it, tilrd says READY=1 once it listens.
    notify_dir = t.scratch("notify")
    notify_dir.chmod(0o777)
    notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
    notify.bind(str(notify_dir / "notify"))
    (notify_dir / "notify").chmod(0o777)
    notify.settimeout(20)
    exec_start = next(line for line in lines if line.startswith("ExecStart="))[len("ExecStart="):]
    started = time.monotonic()
    t.spawn("runuser", "-u", user.name, "--", "env", "-i",
            *[f"{k}={v}" for k, v in user.env(NOTIFY_SOCKET=notify_dir / "notify").items()],
            "sh", "-c", exec_start)
    message = notify.recv(64)
    t.metric("ready_ms", round((time.monotonic() - started) * 1000))
    t.eq(message, b"READY=1", "tilrd tells the service manager it is ready")
    t.eq(user.run("tilr", "ls", "-q", env=env).code, 0, "and it serves the socket then")
    t.eq(len(t.tilrd_daemons()), 1, "one tilrd")

    # NOTIFY_SOCKET is the server's, not its panes'.
    pane_env = user.home / "pane-env"
    r = user.run("tilr", "new", "-d", "--", "sh", "-c", f"env > {pane_env}.tmp && mv {pane_env}.tmp {pane_env}", env=env)
    t.eq(r.code, 0, f"tilr new -d:\n{r.text}")
    t.eventually(pane_env.exists, "the pane ran env")
    seen = pane_env.read_text()
    t.ok("TILR_PANE=" in seen, "the pane's environment")
    t.lacks(seen, "NOTIFY_SOCKET=")
    t.eq(len(t.tilrd_daemons()), 1, "still one tilrd")


@case(spec=["tilr-cli/service-install-with-systemd"])
def with_lingering_off_install_turns_it_on_and_notes_it(t: T):
    """UC-020: lingering off: loginctl enable-linger, the unit notes it for uninstall."""
    user, fake, env = install(t, linger="no")
    r = user.run("tilr", "service", "install", env=env)
    t.eq(r.code, 0, f"tilr service install:\n{r.text}")
    t.ok(f"loginctl enable-linger {user.name}" in fake.called(), "lingering turned on")
    t.has((user.home / ".config/systemd/user/tilr.service").read_text(), "lingering turned on by install")
    t.has(r.err, "lingering turned on")


@case(spec=["tilr-cli/service-install-with-systemd"])
def a_running_tilrd_is_left_alone(t: T):
    """UC-020: a tilrd `tilr` started runs: the unit is enabled, not started; the server is untouched."""
    user, fake, env = install(t)
    t.eq(user.run("tilr", "new", "-d", env=env).code, 0, "tilr new -d")
    daemon = t.tilrd_daemons()[0]
    r = user.run("tilr", "service", "install", env=env)
    t.eq(r.code, 0, f"tilr service install:\n{r.text}")
    t.ok("systemctl --user enable tilr.service" in fake.called(), "enabled")
    t.lacks("\n".join(fake.called()), "--now", "start")
    t.has(r.err, "a tilrd runs already")
    t.eq([d.pid for d in t.tilrd_daemons()], [daemon.pid], "the same server")


@case(spec=["tilr-cli/service-install-leaves-a-unit-or-plist-it-did-not-write#a-unit-it-did-not-write"])
def install_leaves_a_unit_it_did_not_write(t: T):
    """UC-020: a hand-written tilr.service (no `tilr service` mark): install fails naming it, the file is unchanged, systemd is told nothing."""
    user, fake, env = install(t, linger="no")
    unit = user.home / ".config/systemd/user/tilr.service"
    own = "[Unit]\nDescription=my own tilrd\n\n[Service]\nExecStart=/opt/tilrd/tilrd --listen 0.0.0.0:7890\n"
    r = user.run("sh", "-c", f"mkdir -p {unit.parent} && cat > {unit}", input=own)
    t.eq(r.code, 0, "the user's unit is written")
    r = user.run("tilr", "service", "install", env=env)
    t.ok(r.code != 0, f"tilr service install fails (exit {r.code})")
    t.has(r.err, str(unit), "was not written by `tilr service install`: move it away to install")
    t.eq(unit.read_text(), own, "the user's unit is unchanged")
    t.eq(fake.called(), ["systemctl --user show-environment"], "systemd and loginctl are told nothing")
    t.eq(fake.crontab(), None, "no crontab either")
    t.eq(t.tilrd_daemons(), [], "nothing started")
