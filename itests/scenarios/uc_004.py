"""UC-004: a release that cannot be installed leaves nothing: a bad checksum, a download
cut short, an archive cut short (its checksum made of what is left), an archive missing
a program, a program in it that does not run, no release for the platform, a version
named whose archive is not there. Nothing of the release is installed and nothing is
left (no temporary file in the home or in /tmp, no directory made for it), and the
installer exits non-zero; an installation already there, and its running server, stay
as they were. A reload request that fails after installation instead keeps both
new programs, exits non-zero and reports the request failure.

The installer (scripts/install.sh) downloads the archive into a directory of mktemp,
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


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/the-release-is-checked-in-a-temporary-directory-first",
])
def a_bad_checksum_installs_nothing(t: T):
    """UC-004: the archive's checksum does not match: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    archive = cdn.publish(t.manifest()["version"])
    cdn.checksum(archive).write_text("0" * 64 + "\n")
    install_fails(t, user, url, env, "checksum mismatch, nothing installed")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/the-release-is-checked-in-a-temporary-directory-first",
])
def a_download_cut_short_installs_nothing(t: T):
    """UC-004: the connection drops halfway through the archive: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    archive = cdn.publish(t.manifest()["version"])
    cdn.cut[cdn.path(archive)] = archive.stat().st_size // 2
    install_fails(t, user, url, env, "download failed, nothing installed")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/the-release-is-checked-in-a-temporary-directory-first",
])
def an_archive_cut_short_installs_nothing(t: T):
    """UC-004: an archive cut in half, its checksum made of the half: unpacking fails, nothing installed."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    archive = cdn.publish(version)
    data = archive.read_bytes()
    archive.write_bytes(data[: len(data) // 2])
    cdn.write_checksum(archive)
    install_fails(t, user, url, env, f"the archive of {version} is damaged or lacks tilrd or tilr, nothing installed")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/both-programs-are-installed-or-neither",
])
def an_archive_without_tilr_installs_nothing(t: T):
    """UC-004: the archive has tilrd but no tilr: tilrd is not installed either."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    cdn.publish(version, client=None)
    install_fails(t, user, url, env, f"the archive of {version} is damaged or lacks tilrd or tilr, nothing installed")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/both-programs-are-installed-or-neither",
])
def a_program_that_does_not_run_installs_nothing(t: T):
    """UC-004: tilr fails its version check: both checks precede any installation directory."""
    user, cdn, url, env = setup(t)
    version = t.manifest()["version"]
    evidence = t.scratch("version-checks")
    evidence.chmod(0o777)
    temporary = evidence / "custom-tmp"
    temporary.mkdir(mode=0o777)
    temporary.chmod(0o777)
    env["TMPDIR"] = str(temporary)
    env["TILR_BIN_DIR"] = str(user.home / ".local/bin")
    programs = {}
    for name, status in (("tilrd", 0), ("tilr", 1)):
        program = evidence / name
        program.write_text(
            '#!/bin/sh\n'
            f'printf "%s\\n" "$0" "$1" > "{evidence}/{name}.called"\n'
            f'if [ -e "$TILR_BIN_DIR" ]; then echo exists; else echo absent; fi '
            f'>> "{evidence}/{name}.called"\n'
            f'exit {status}\n'
        )
        program.chmod(0o755)
        programs[name] = program
    cdn.publish(version, server=programs["tilrd"], client=programs["tilr"])
    install_fails(t, user, url, env, f"tilr {version} does not run on this system", "nothing installed")
    directories = []
    for name in programs:
        called, argument, destination = (evidence / f"{name}.called").read_text().splitlines()
        path = Path(called)
        t.eq(path.name, name, "the unpacked program is checked")
        t.eq(path.parent.name, "release", "checked in the temporary release directory")
        t.ok(path.is_relative_to(temporary), "checked under the configured TMPDIR")
        t.eq((argument, destination), ("--version", "absent"), "checked before making the install directory")
        directories.append(path.parent)
    t.eq(directories[0], directories[1], "both programs checked in the same temporary directory")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/no-release-for-the-platform-is-a-clear-error",
])
def no_release_for_the_platform_installs_nothing(t: T):
    """UC-004: the CDN has no release for this platform: a clear error, nothing installed."""
    user, cdn, url, env = setup(t)
    other = cdn.dir / "releases" / "linux-riscv64" / "latest"
    other.parent.mkdir(parents=True)
    other.write_text("9.9.9\n")
    install_fails(t, user, url, env, f"no release for linux {cdn.platform.split('-', 1)[1]}")


@case(spec=[
    "installation/a-release-that-cannot-be-installed-leaves-nothing",
    "installation/no-release-for-the-platform-is-a-clear-error",
])
def a_latest_version_without_its_archive_installs_nothing(t: T):
    """UC-004: `latest` names a version whose archive is not there (yet): nothing installed."""
    user, cdn, url, env = setup(t)
    cdn.latest("9.9.9-missing")
    install_fails(t, user, url, env, f"no release 9.9.9-missing for linux {cdn.platform.split('-', 1)[1]}",
                  "nothing installed")


@case(spec=["installation/a-failed-update-leaves-the-installation-and-its-server-alone"])
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


@case(spec=["installation/a-failed-reload-request-keeps-the-installed-release"])
def a_failed_reload_request_keeps_the_installed_release(t: T):
    """UC-004: after installation, a refused reload request keeps both new files."""
    user, cdn, url, env = setup(t)
    evidence = t.scratch("reload-request")
    temporary = t.scratch("reload-tmp")
    temporary.chmod(0o777)
    env["TMPDIR"] = str(temporary)
    server = evidence / "tilrd"
    client = evidence / "tilr"
    server.write_text('#!/bin/sh\ncase "$1" in --version) echo "tilrd release";; reload) exit 1;; esac\n')
    client.write_text('#!/bin/sh\necho "tilr release"\n')
    for program in (server, client):
        program.chmod(0o755)
    version = t.manifest()["version"]
    cdn.publish(version, server=server, client=client)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.ok(r.code != 0, "reload request failure exits nonzero")
    t.has(r.err, f"installed tilr {version}, but could not request reload")
    t.ok("the running tilrd reloads itself" not in r.err, "no reload success reported")
    installed = user.home / ".local/bin"
    for program in (server, client):
        t.eq((installed / program.name).read_bytes(), program.read_bytes(), "new program retained")
    t.eq(sorted(p.name for p in installed.iterdir()), ["tilr", "tilrd"], "no staging files")
    t.eq(list(temporary.iterdir()), [], "temporary download directory removed")
    t.eq(t.tilrd_daemons(), [], "no server started")
