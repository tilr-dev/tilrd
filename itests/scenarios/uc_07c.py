"""UC-07C: deploy over real SSH, preserving a running pane on upgrade."""

import shlex

from tt import T, case


SPEC = "tilr-cli/tilr-deploy-installs-and-starts-tilrd-over-ssh"


def setup(t: T):
    laptop, remote = t.host("laptop"), t.host("remote")
    laptop.sh("mkdir -p /root/.ssh && chmod 700 /root/.ssh && "
              "ssh-keygen -q -t ed25519 -N '' -f /root/.ssh/id_ed25519", check=True)
    public = laptop.run("cat", "/root/.ssh/id_ed25519.pub", check=True).out.strip()
    remote.sh("mkdir -p /run/sshd /root/.ssh && chmod 700 /root/.ssh && "
              "ssh-keygen -A >/dev/null && "
              f"echo {shlex.quote(public)} >/root/.ssh/authorized_keys && "
              "chmod 600 /root/.ssh/authorized_keys && "
              # A genuinely fresh host, with no system installation to fall back to.
              "rm -f /usr/local/bin/tilr /usr/local/bin/tilrd && "
              "printf 'PS1=\"$ \"\n' >/root/.bashrc && "
              "/usr/sbin/sshd </dev/null >/dev/null 2>&1", check=True)
    config = (f"Host target\n  HostName {remote.ip}\n  User root\n"
              "  BatchMode yes\n  StrictHostKeyChecking no\n"
              "  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n")
    laptop.sh(f"printf %s {shlex.quote(config)} >/root/.ssh/config && chmod 600 /root/.ssh/config", check=True)
    return laptop, remote


def deployed(t: T, laptop, *args):
    result = laptop.run("tilr", "deploy", *args, "target")
    t.eq(result.code, 0, f"deploy succeeds:\n{result.text}")
    lines = result.out.strip().splitlines()
    t.eq(lines[:2], ["ssh -t target ~/.local/bin/tilr",
                     "ssh -N -L 7890:127.0.0.1:7890 target"], "ready connection commands")
    t.ok(len(lines) == 3 and len(lines[2]) >= 32, "remote token printed")
    return lines[2]


@case(hosts=["laptop", "remote"], services=["cdn"], spec=[
    SPEC + "#a-fresh-host", SPEC + "#a-server-already-running-there",
])
def deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh(t: T):
    """Fresh install and a newer CDN release keep a live shell and client."""
    laptop, remote = setup(t)
    version, platform = t.manifest()["version"], t.manifest()["platform"]
    archive = f"/tmp/tilr-{version}-{platform}.tar.gz"
    laptop.sh(f"tar -czf {archive} -C /usr/local/bin tilr tilrd && sha256sum {archive} >{archive}.sha256", check=True)
    token = deployed(t, laptop, "--archive", archive)
    t.eq(remote.run("cat", "/root/.local/state/tilr/token", check=True).out.strip(), token, "printed token belongs to remote")
    for name in ("tilr", "tilrd"):
        t.eq(remote.run(f"/root/.local/bin/{name}", "--version", check=True).out.split()[1],
             t.manifest()["version"], f"{name} is the current release")
    d = remote.driver("deployed")
    d.connect("ws://127.0.0.1:7890", token=token)
    pane = d.shell_pane()
    before = d.value(pane, "$$")
    d.line(pane, "export DEPLOY_MARK=kept")
    t.eq(d.value(pane, "$DEPLOY_MARK"), "kept", "live shell has state")
    since = d.mark()
    t.eq(deployed(t, laptop, "--cdn", t.service("cdn")), token, "upgrade keeps token")
    d.wait_event("disconnected", since=since)
    d.cmd("reconnect", timeout_ms=20_000)
    t.eq(d.value(pane, "$$ $DEPLOY_MARK"), before + " kept", "same shell and environment survive deploy")
    for name in ("tilr", "tilrd"):
        t.eq(remote.run(f"/root/.local/bin/{name}", "--version", check=True).out.split()[1],
             t.manifest()["next_version"], f"{name} is the newer release")
    remote.run("/root/.local/bin/tilr", "kill-server", check=True)


@case(hosts=["laptop", "remote"], spec=[SPEC + "#a-damaged-archive"])
def a_failed_deploy_leaves_the_remote_as_it_was(t: T):
    """Bad checksum fails locally before any install is sent."""
    laptop, remote = setup(t)
    platform = t.manifest()["platform"]
    path = f"/tmp/tilr-9.9.9-{platform}.tar.gz"
    laptop.sh(f"printf damaged >{path}; printf '%064d' 0 >{path}.sha256", check=True)
    result = laptop.run("tilr", "deploy", "--archive", path, "target")
    t.ok(result.code != 0, "damaged archive fails")
    t.has(result.err, "checksum mismatch")
    t.eq(remote.sh("test ! -e /root/.local/bin/tilr && test ! -e /root/.local/bin/tilrd").code,
         0, "no remote installation")
