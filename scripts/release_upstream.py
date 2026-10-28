#!/usr/bin/env python3
"""Check fetched PR objects without checking out or executing candidate code."""

import argparse
import json
import subprocess
import sys


def git(repo, *args, input=None):
    return subprocess.run(
        ["git", "-C", str(repo), *args], input=input, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
    ).stdout.strip()


def check(repo, event):
    if git(repo, "rev-parse", "--is-shallow-repository") != "false":
        raise ValueError("full Git history is required")
    pr = event["pull_request"]
    target = pr["base"]["ref"]
    if not target.startswith("release/"):
        raise ValueError("expected a release/** target")
    # Bind fetched snapshots to the event. A newer push must receive its own run.
    for ref, expected in [("refs/inspection/base", pr["base"]["sha"]),
                          ("refs/inspection/head", pr["head"]["sha"])]:
        if git(repo, "rev-parse", ref) != expected:
            raise ValueError(f"{ref} does not match the pull request event; rerun the check")
    git(repo, "rev-parse", "refs/inspection/main^{commit}")
    git(repo, "merge-base", "refs/inspection/base", "refs/inspection/head")
    commits = git(repo, "rev-list", "--reverse",
                  "refs/inspection/base..refs/inspection/head").splitlines()
    # Include all main history: the source patch may predate the release branch.
    patches = git(repo, "log", "--no-merges", "--pretty=format:commit %H", "-p",
                  "--binary", "refs/inspection/main")
    upstream = {line.split()[0] for line in
                git(repo, "patch-id", "--stable", input=patches).splitlines()}
    rejected = []
    for commit in commits:
        ancestors = git(repo, "rev-list", "-1", commit, "--not", "refs/inspection/main")
        if not ancestors:
            continue
        parents = git(repo, "rev-list", "--parents", "-n", "1", commit).split()[1:]
        patch = git(repo, "show", "--pretty=format:commit %H", "--binary", commit)
        identity = git(repo, "patch-id", "--stable", input=patch).split()
        if len(parents) != 1 or not identity or identity[0] not in upstream:
            rejected.append(commit)
    if rejected:
        raise ValueError("release-upstream: commits absent from main: " + ", ".join(rejected)
                         + ". Land each change in main first, then cherry-pick it into " + target)
    print(f"release-upstream: {len(commits)} commit(s) verified against main")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--event", required=True)
    args = parser.parse_args()
    try:
        with open(args.event, encoding="utf-8") as source:
            check(args.repo, json.load(source))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"release-upstream failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
