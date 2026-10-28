#!/usr/bin/env python3
"""Reject release tags older than the workspace package version without editing Cargo files."""

import re
import sys
import tomllib
from pathlib import Path


def numeric(version):
    match = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-rc\.[1-9][0-9]*)?", version)
    if match is None:
        raise ValueError(f"invalid release version: {version}")
    return tuple(map(int, match.groups()))


def main():
    if len(sys.argv) != 2 or not sys.argv[1].startswith("v"):
        raise ValueError("expected a vX.Y.Z or vX.Y.Z-rc.N tag")
    tag = sys.argv[1][1:]
    package = tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    if numeric(tag) < numeric(package):
        raise ValueError(f"release tag {tag} is older than Cargo version {package}")
    print(f"release version {tag} >= Cargo version {package}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError) as error:
        sys.exit(str(error))
