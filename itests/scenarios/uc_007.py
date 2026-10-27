"""UC-007: install both programs by hand from the CDN layout, then attach with tilr."""

import hashlib
import os
import tarfile
from pathlib import Path

from tt import USER_PATH, T, case


@case(spec=["installation/manual-install-from-the-cdn"])
def a_manual_install_from_the_cdn_layout_runs(t: T):
    """The README's latest/archive/tar steps install a working client and server."""
    t.need("procfs")
    user = t.user()
    user.prompt()
    cdn = t.cdn()
    version = t.manifest()["version"]
    archive = cdn.publish(version)
    bin = user.home / ".local/bin"
    path = f"{bin}:{USER_PATH}"

    result = user.sh(
        "set -euo pipefail\n"
        "mkdir -p \"$HOME/.local/bin\"\n"
        f"platform={cdn.platform}\n"
        f"version=$(curl -fsSL \"{cdn.url}/releases/$platform/latest\")\n"
        f"curl -fsSL \"{cdn.url}/releases/$version/tilr-$version-$platform.tar.gz\" |\n"
        "  tar -xz -C \"$HOME/.local/bin\" tilrd tilr\n",
        env={"PATH": path},
    )
    t.eq(result.code, 0, f"manual download and extraction:\n{result.text}")
    t.eq(cdn.requests, [f"/releases/{cdn.platform}/latest", cdn.path(archive)],
         "latest and its archive were fetched from the CDN")

    with tarfile.open(archive) as tar:
        released = {name: hashlib.sha256(tar.extractfile(name).read()).hexdigest()
                    for name in ("tilrd", "tilr")}
    for name in ("tilrd", "tilr"):
        installed = bin / name
        t.ok(installed.is_file(), f"{name} was unpacked")
        t.eq(hashlib.sha256(installed.read_bytes()).hexdigest(), released[name],
             f"{name} matches the downloaded archive")
        st = installed.stat()
        t.eq(st.st_mode & 0o777, 0o755, f"{name} is executable")
        t.eq(st.st_uid, user.uid, f"{name} belongs to the installing user")

    login_env = user.env(PATH=path)
    client = t.session("env", "-i", *(f"{key}={value}" for key, value in login_env.items()),
                       str(bin / "tilr"), user=user.uid, name="installed tilr")
    client.prompt()
    daemons = t.tilrd_daemons()
    t.eq(len(daemons), 1, "the installed client started one server")
    t.has(daemons[0].cmdline, str(bin / "tilrd"))
    t.eq(os.stat(f"/proc/{daemons[0].pid}").st_uid, user.uid,
         "the installed server runs as the installing user")
    user.run(str(bin / "tilr"), "kill-server", env={"PATH": path}, check=True)
    client.expect(r"\[tilrd stopped\]")
    t.eq(client.wait_exit(), 0, "the attached client exits after stopping the server")
