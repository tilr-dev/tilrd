"""UC-001: `curl tilr.sh | sh` without root: the release archive is downloaded into a
temporary directory, its checksum checked, tilrd and tilr from it land in ~/.local/bin,
nothing else is left anywhere, the hints are printed, and `tilr` then starts the
installed tilrd.

The installer is scripts/install.sh, the one tilr.sh serves, with a release
built from this tree (tilr-<version>-<platform>.tar.gz and its .sha256) by a CDN of the
case's own; it runs as a user made for the case, with a login's environment and
~/.local/bin on PATH.
"""

import hashlib
import os
import tarfile
from pathlib import Path

from tt import USER_PATH, T, case, changes, snapshot


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@case(spec=[
    "installation/both-executables-go-into-local-bin-without-root",
    "installation/the-installer-reads-the-cdn-layout",
    "installation/the-installer-sets-up-no-service",
    "release-process/the-cdn-layout-is-one-archive-per-platform",
])
def the_installer_puts_tilrd_and_tilr_in_local_bin_without_root(t: T):
    """UC-001: no root: both executables in ~/.local/bin, checksums checked, the version printed."""
    user = t.user()
    cdn = t.cdn()
    version = t.manifest()["version"]
    archive = cdn.publish(version)
    url = cdn.installer()
    bin = user.home / ".local/bin"
    system = {d: snapshot(Path(d)) for d in ("/usr/local/bin", "/etc")}
    tmp = snapshot(Path("/tmp"), skip=("tt",))
    home = snapshot(user.home)

    r = user.sh(f"curl -fsSL {url} | sh", env={"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"})
    t.eq(r.code, 0, f"the installer's exit status:\n{r.text}")
    t.eq([cdn.path(archive) + ".sha256", cdn.path(archive)],
         [p for p in cdn.requests if p.startswith(f"/releases/{version}/")], "the archive and its checksum fetched")
    with tarfile.open(archive) as tar:
        released = {m.name: hashlib.sha256(tar.extractfile(m).read()).hexdigest() for m in tar.getmembers()}
    for name in ("tilrd", "tilr"):
        path = bin / name
        t.ok(path.is_file(), f"{path} is installed")
        t.eq(sha256(path), released[name], f"{name} is the one in the release")
        st = path.stat()
        t.eq(oct(st.st_mode & 0o777), "0o755", f"{name} is executable")
        t.eq(st.st_uid, user.uid, f"{name} belongs to the user")
        t.has(user.run(str(path), "--version").out, version)
    t.has(r.err, f"downloading tilr {version}", f"installed tilrd and tilr {version} in {bin}")
    t.has(r.err, "open a shell: tilr", "start tilrd at boot: tilr service install", "tilr token", "tilr cloud login")
    t.lacks(r.text, "sudo", "not on your PATH")
    t.eq(sorted(line.split(":")[0] for line in changes(home, snapshot(user.home))),
         ["+ .local", "+ .local/bin", "+ .local/bin/tilr", "+ .local/bin/tilrd"], "only the executables are new in the home")
    t.eq(changes(tmp, snapshot(Path("/tmp"), skip=("tt",))), [], "no temporary file is left in /tmp")
    for d, before in system.items():
        t.eq(changes(before, snapshot(Path(d))), [], f"nothing changed in {d}")

    # The installed tilr starts the installed tilrd, as the user.
    t.eq(user.run(str(bin / "tilr"), "new", "-d").code, 0, "tilr new -d")
    daemons = t.tilrd_daemons()
    t.eq(len(daemons), 1, "one tilrd")
    t.has(daemons[0].cmdline, str(bin / "tilrd"))
    t.eq(os.stat(f"/proc/{daemons[0].pid}").st_uid, user.uid, "tilrd runs as the user")
    t.has(user.run(str(bin / "tilr"), "version").out, f"tilr {version}", f"tilrd {version}")
    user.run(str(bin / "tilr"), "kill-server")
