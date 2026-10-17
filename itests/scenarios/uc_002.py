"""UC-002: ~/.local/bin is not on PATH: the installer says what to add and edits nothing
on its own (no ~/.profile, ~/.bashrc or anything else of the user's).

What the home holds before and after is compared file by file: only ~/.local/bin and the
two executables may be new.
"""

from pathlib import Path

from tt import USER_PATH, T, case, changes, snapshot


@case()
def without_local_bin_on_path_the_installer_says_what_to_add_and_edits_nothing(t: T):
    """UC-002: the hint names ~/.local/bin and PATH; nothing of the user's is edited."""
    user = t.user()
    cdn = t.cdn()
    cdn.publish(t.manifest()["version"])
    url = cdn.installer()
    bin = user.home / ".local/bin"
    before = snapshot(user.home)
    etc = snapshot(Path("/etc"))

    r = user.sh(f"curl -fsSL {url} | sh", env={"TILR_CDN": cdn.url, "PATH": USER_PATH})
    t.eq(r.code, 0, f"the installer's exit status:\n{r.text}")
    t.has(r.err, f"{bin} is not on your PATH")
    added = changes(before, snapshot(user.home))
    t.eq(sorted(line.split(":")[0] for line in added),
         ["+ .local", "+ .local/bin", "+ .local/bin/tilr", "+ .local/bin/tilrd"],
         "only the executables (and their directory) are new in the home")
    t.eq(changes(etc, snapshot(Path("/etc"))), [], "nothing changed in /etc")
