"""UC-004: a release that cannot be installed leaves nothing: a bad checksum, a download
cut short, no release for the platform. Nothing of the release is installed and nothing
is left in the home (no temporary file, no directory made for it); an installation
already there, and its running server, stay as they were.

Observed (installer/install.sh): install_binary makes ~/.local/bin (`mkdir -p
"$BIN_DIR"`, line 69) before the download is checked, so a failed first install
leaves an empty ~/.local and ~/.local/bin (red: the bad checksum and the cut download).
tilrd is put in place before tilr is fetched (main, lines 226-227) and a tilr that fails
is "said and skipped" (lines 102-120): a bad tilr leaves the new tilrd installed and the
installer exits 0 (red: a bad client checksum).
"""

from tt import NEXT_EXE, USER_PATH, T, case, changes, snapshot


def setup(t: T):
    user = t.user()
    cdn = t.cdn()
    url = cdn.installer()
    bin = user.home / ".local/bin"
    env = {"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"}
    return user, cdn, url, env


def nothing_installed(t: T, user, before: dict, r, *said: str):
    t.ok(r.code != 0, f"the installer fails:\n{r.text}")
    t.has(r.err, *said)
    t.check("nothing is installed or left in the home",
            lambda: t.eq(changes(before, snapshot(user.home)), [], "what changed in the home"))
    t.eq(t.tilrd_daemons(), [], "no server started")


@case()
def a_bad_checksum_installs_nothing(t: T):
    """UC-004: tilrd's checksum does not match: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    files = cdn.publish(t.manifest()["version"])
    files["tilrd"].with_name(files["tilrd"].name + ".sha256").write_text("0" * 64 + "\n")
    before = snapshot(user.home)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    nothing_installed(t, user, before, r, "checksum mismatch, nothing installed")


@case()
def a_download_cut_short_installs_nothing(t: T):
    """UC-004: the connection drops halfway through tilrd: nothing installed, nothing left."""
    user, cdn, url, env = setup(t)
    files = cdn.publish(t.manifest()["version"])
    cdn.cut[cdn.path(files["tilrd"])] = files["tilrd"].stat().st_size // 2
    before = snapshot(user.home)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    nothing_installed(t, user, before, r, "download failed")


@case()
def no_release_for_the_platform_installs_nothing(t: T):
    """UC-004: the CDN has no release for this platform: a clear error, nothing installed."""
    user, cdn, url, env = setup(t)
    other = cdn.dir / "releases" / "linux-riscv64" / "latest"
    other.parent.mkdir(parents=True)
    other.write_text("9.9.9\n")
    before = snapshot(user.home)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    nothing_installed(t, user, before, r, f"no release for linux {cdn.platform.split('-', 1)[1]}")


@case()
def a_bad_client_checksum_installs_neither(t: T):
    """UC-004: tilr's checksum does not match: tilrd is not installed either."""
    user, cdn, url, env = setup(t)
    files = cdn.publish(t.manifest()["version"])
    files["tilr"].with_name(files["tilr"].name + ".sha256").write_text("0" * 64 + "\n")
    before = snapshot(user.home)
    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    nothing_installed(t, user, before, r, "checksum mismatch for tilr")


@case()
def a_broken_update_leaves_the_installation_and_its_server_alone(t: T):
    """UC-004: a new release with a bad checksum over a running install: files and server as they were."""
    user, cdn, url, env = setup(t)
    m = t.manifest()
    cdn.publish(m["version"])
    t.eq(user.sh(f"curl -fsSL {url} | sh", env=env).code, 0, "the first install")
    bin = user.home / ".local/bin"
    t.eq(user.run(str(bin / "tilr"), "new", "-d").code, 0, "tilr new -d")
    daemon = t.tilrd_daemons()[0]
    files = cdn.publish(m["next_version"], server=NEXT_EXE)
    files["tilrd"].with_name(files["tilrd"].name + ".sha256").write_text("0" * 64 + "\n")
    # What the running server keeps in its state directory is its own business.
    before = snapshot(user.home, skip=(".local/state",))

    r = user.sh(f"curl -fsSL {url} | sh", env=env)
    t.ok(r.code != 0, f"the installer fails:\n{r.text}")
    t.has(r.err, "checksum mismatch, nothing installed")
    t.eq(changes(before, snapshot(user.home, skip=(".local/state",))), [], "what changed in the home")
    t.eq([d.pid for d in t.tilrd_daemons()], [daemon.pid], "the same server")
    t.has(user.run(str(bin / "tilr"), "version").out, f"tilrd {m['version']}")
