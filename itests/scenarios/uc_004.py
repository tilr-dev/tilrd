"""UC-004: a release that cannot be installed leaves nothing: a bad checksum, a download
cut short, an archive cut short (its checksum made of what is left), an archive missing
a program, a program in it that does not run, no release for the platform, a version
named whose archive is not there. Nothing of the release is installed and nothing is
left (no temporary file in the home or in /tmp, no directory made for it), and the
installer exits non-zero; an installation already there, and its running server, stay
as they were.

The installer (installer/install.sh) downloads the archive into a directory of mktemp,
checks and unpacks it there and runs both programs before it makes ~/.local/bin.
"""

from pathlib import Path

from tt import CLIENT_NEXT_EXE, NEXT_EXE, USER_PATH, T, case, changes, snapshot


def setup(t: T):
    user = t.user()
    cdn = t.cdn()
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"}
    return user, cdn, url, env


def install_fails(t: T, user, url: str, env: dict, *said: str):
    """Run the installer where nothing is installed yet: it fails, saying `said`, and
    leaves nothing in the home or in /tmp."""
    home = snapshot(user.home)
    tmp = snapshot(Path("/tmp"), skip=("tt",))
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.ok(r.code != 0, f"the installer fails:\n{r.text}")
    t.has(r.err, *said)
    t.check("nothing is installed or left in the home",
            lambda: t.eq(changes(home, snapshot(user.home)), [], "what changed in the home"))
    t.check("nothing is left in /tmp",
            lambda: t.eq(changes(tmp, snapshot(Path("/tmp"), skip=("tt",))), [], "what changed in /tmp"))
    t.eq(t.tilrd_daemons(), [], "no server started")


@case()
def a_bad_checksum_installs_nothing(t: T):
    """UC-004: the archive's checksum does not match: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    archive = cdn.publish(t.manifest()["version"])
    cdn.checksum(archive).write_text("0" * 64 + "\n")
    install_fails(t, user, url, env, "checksum mismatch, nothing installed")


@case()
def a_download_cut_short_installs_nothing(t: T):
    """UC-004: the connection drops halfway through the archive: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    archive = cdn.publish(t.manifest()["version"])
    cdn.cut[cdn.path(archive)] = archive.stat().st_size // 2
    install_fails(t, user, url, env, "download failed, nothing installed")


@case()
def an_archive_cut_short_installs_nothing(t: T):
    """UC-004: an archive cut in half, its checksum made of the half: unpacking fails, nothing installed."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    archive = cdn.publish(version)
    data = archive.read_bytes()
    archive.write_bytes(data[: len(data) // 2])
    cdn.write_checksum(archive)
    install_fails(t, user, url, env, f"the archive of {version} is damaged or lacks tilrd or tilr, nothing installed")


@case()
def an_archive_without_tilr_installs_nothing(t: T):
    """UC-004: the archive has tilrd but no tilr: tilrd is not installed either."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    cdn.publish(version, client=None)
    install_fails(t, user, url, env, f"the archive of {version} is damaged or lacks tilrd or tilr, nothing installed")


@case()
def a_program_that_does_not_run_installs_nothing(t: T):
    """UC-004: tilr in the archive is not a program: tilrd is not installed either."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    broken = t.scratch("broken") / "tilr"
    broken.write_bytes(b"\x7fELF not really\n")
    cdn.publish(version, client=broken)
    install_fails(t, user, url, env, f"tilr {version} does not run on this system", "nothing installed")


@case()
def no_release_for_the_platform_installs_nothing(t: T):
    """UC-004: the CDN has no release for this platform: a clear error, nothing installed."""
    user, cdn, url, env = setup(t)
    other = cdn.dir / "releases" / "linux-riscv64" / "latest"
    other.parent.mkdir(parents=True)
    other.write_text("9.9.9\n")
    install_fails(t, user, url, env, f"no release for linux {cdn.platform.split('-', 1)[1]}")


@case()
def a_latest_version_without_its_archive_installs_nothing(t: T):
    """UC-004: `latest` names a version whose archive is not there (yet): nothing installed."""
    user, cdn, url, env = setup(t)
    cdn.latest("9.9.9-missing")
    install_fails(t, user, url, env, f"no release 9.9.9-missing for linux {cdn.platform.split('-', 1)[1]}",
                  "nothing installed")


@case()
def a_broken_update_leaves_the_installation_and_its_server_alone(t: T):
    """UC-004: a new release with a bad checksum, then one without tilr, over a running install: all as it was."""
    user, cdn, url, env = setup(t)
    m = t.manifest()
    cdn.publish(m["version"])
    t.eq(user.sh(f"curl -fsSL {url} | sh", env=env).code, 0, "the first install")
    bin = user.home / ".local/bin"
    t.eq(user.run(str(bin / "tilr"), "new", "-d").code, 0, "tilr new -d")
    daemon = t.tilrd_daemons()[0]
    # What the running server keeps in its state directory is its own business.
    before = snapshot(user.home, skip=(".local/state",))

    archive = cdn.publish(m["next_version"], server=NEXT_EXE, client=CLIENT_NEXT_EXE)
    cdn.checksum(archive).write_text("0" * 64 + "\n")
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.ok(r.code != 0, f"the installer fails:\n{r.text}")
    t.has(r.err, "checksum mismatch, nothing installed")

    cdn.publish(m["next_version"], server=NEXT_EXE, client=None)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.ok(r.code != 0, f"the installer fails:\n{r.text}")
    t.has(r.err, "lacks tilrd or tilr, nothing installed")

    t.eq(changes(before, snapshot(user.home, skip=(".local/state",))), [], "what changed in the home")
    t.eq([d.pid for d in t.tilrd_daemons()], [daemon.pid], "the same server")
    t.has(user.run(str(bin / "tilr"), "version").out, f"tilr {m['version']}", f"tilrd {m['version']}")
