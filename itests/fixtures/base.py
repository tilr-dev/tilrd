"""Files the live-software cases open, and a server state directory with its token."""

import os
import re

from tt import T, fixture


@fixture()
def files(t: T):
    """/fx/big.txt: 200000 lines `line N`, for less; /fx/notes: a directory for editors."""
    with open(t.root / "big.txt", "w") as f:
        f.writelines(f"line {n}\n" for n in range(1, 200_001))
    (t.root / "notes").mkdir()


@files.check
def _(t: T):
    lines = (t.root / "big.txt").read_text().splitlines()
    t.eq((len(lines), lines[0], lines[-1]), (200_000, "line 1", "line 200000"), "big.txt")
    t.eq(list((t.root / "notes").iterdir()), [], "notes is empty")


@fixture(requires=["files"], tilr=True)
def server_state(t: T):
    """/fx/state: TILR_STATE_DIR with the access token made by `tilrd token`."""
    t.run("tilrd", "token", env={"TILR_STATE_DIR": str(t.root / "state")}, check=True)


@server_state.check
def _(t: T):
    token = (t.root / "state" / "token").read_text().strip()
    t.ok(re.fullmatch(r"[0-9a-f]{64}", token), "the token is 64 hex digits")
    t.eq(oct(os.stat(t.root / "state" / "token").st_mode & 0o777), "0o600", "token mode")
    printed = t.run("tilrd", "token", env={"TILR_STATE_DIR": str(t.root / "state")}, check=True).out.strip()
    t.eq(printed, token, "tilrd token prints the stored token")
