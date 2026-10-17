"""UC-001: `curl tilr.sh | sh` without root: tilrd and tilr land in ~/.local/bin, their
checksums checked, the version is printed, and `tilr` then starts the installed tilrd.

The installer is the sh repository's (itests/installer/install.sh), served with a release
built from this tree (tilrd, tilr, their .sha256) by a CDN of the case's own; it runs as a
user made for the case, with a login's environment and ~/.local/bin on PATH.
"""

import hashlib
import os
from pathlib import Path

from tt import USER_PATH, T, case, changes, snapshot


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@case()
def the_installer_puts_tilrd_and_tilr_in_local_bin_without_root(t: T):
    """UC-001: no root: both executables in ~/.local/bin, checksums checked, the version printed."""
    user = t.user()
    cdn = t.cdn()
    version = t.manifest()["version"]
    files = cdn.publish(version)
    url = cdn.installer()
    bin = user.home / ".local/bin"
    system = {d: snapshot(Path(d)) for d in ("/usr/local/bin", "/etc")}

    r = user.sh(f"curl -fsSL {url} | sh", env={"TILR_CDN": cdn.url, "PATH": f"{bin}:{USER_PATH}"})
    t.eq(r.code, 0, f"the installer's exit status:\n{r.text}")
    for name in ("tilrd", "tilr"):
        path = bin / name
        t.ok(path.is_file(), f"{path} is installed")
        t.eq(sha256(path), sha256(files[name]), f"{name} is the release")
        st = path.stat()
        t.eq(oct(st.st_mode & 0o777), "0o755", f"{name} is executable")
        t.eq(st.st_uid, user.uid, f"{name} belongs to the user")
        t.ok(f"/releases/{version}/{name}-{cdn.platform}.sha256" in cdn.requests,
             f"the installer fetched {name}'s checksum")
        t.has(user.run(str(path), "--version").out, version)
    t.has(r.err, f"downloading tilrd {version}", f"downloading tilr {version}")
    t.lacks(r.text, "sudo", "not on your PATH")
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
