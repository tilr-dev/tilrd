#!/usr/bin/env bash
# Called from the released tilrd tag after its signed GitHub assets are uploaded.
set -euo pipefail

if [[ ${RELEASE_CREATED:-false} != true || ${IS_RC:-false} == true ]]; then
  echo 'No stable release created; desktop pin unchanged.'
  exit 0
fi
if [[ ! ${TAG:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'Expected a stable vX.Y.Z release tag' >&2
  exit 1
fi
source_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
protocol=$(python3 - "$source_root/crates/tilr-proto/src/lib.rs" <<'PY'
import re
import sys
from pathlib import Path
values = re.findall(r'^pub const PROTOCOL_VERSION: u32 = ([0-9]+);$', Path(sys.argv[1]).read_text(), re.M)
if len(values) != 1 or not 0 < int(values[0]) <= 4294967295:
    sys.exit('Missing or invalid released PROTOCOL_VERSION')
print(int(values[0]))
PY
)
version=${TAG#v}
repo=tilr-dev/desktop-mac
branch=bot/bump-tilrd
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh auth setup-git
gh repo clone "$repo" "$work/desktop" -- --branch main --single-branch
cd "$work/desktop"
# Lease against the branch observed now, including absence on the first release.
previous=$(git ls-remote --heads origin "refs/heads/$branch" | cut -f1)
# Read only an open proposal: a leftover closed PR branch is not a pending pin.
number=$(gh pr list --repo "$repo" --base main --head "$branch" --state open --json number --jq '.[0].number // empty')
if [[ -n $number ]]; then
  if [[ -z $previous ]]; then
    echo 'Open desktop pin pull request has no branch' >&2
    exit 1
  fi
  git fetch origin "$previous"
  git show "$previous:server-release.json" > "$work/proposed-pin.json"
fi
git checkout -b "$branch" origin/main
python3 - "$version" "$protocol" "$work/proposed-pin.json" <<'PY'
import json
import re
import sys
from pathlib import Path
path = Path('server-release.json')
pin = json.loads(path.read_text())
if type(pin.get('build')) is not int or pin['build'] < 1:
    sys.exit('Desktop pin must have a positive artifact revision')
def version(value):
    if not isinstance(value, str) or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', value):
        sys.exit('Desktop pin must have a stable numeric version')
    return tuple(map(int, value.split('.')))

released = version(sys.argv[1])
current = [('main', pin)]
proposed = Path(sys.argv[3])
if proposed.exists():
    current.append(('open pull request', json.loads(proposed.read_text())))
for source, existing in current:
    if released <= version(existing.get('version')):
        print(f"Released {sys.argv[1]} is not newer than desktop {source} pin {existing['version']}; skipping.")
        sys.exit(0)
pin.update(version=sys.argv[1], protocolVersion=int(sys.argv[2]))
path.write_text(json.dumps(pin, indent=2) + '\n')
PY
if git diff --quiet -- server-release.json; then
  echo 'Desktop pin unchanged.'
  exit 0
fi
title="chore: bump tilrd to $version"
git config user.name 'tilr-bot[bot]'
git config user.email 'tilr-bot[bot]@users.noreply.github.com'
git add server-release.json
git -c commit.gpgsign=false commit -m "$title"
git push "--force-with-lease=refs/heads/$branch:$previous" origin "HEAD:refs/heads/$branch"
body="Pin the signed tilrd $TAG GitHub Release (protocol $protocol). Artifact revision is retained from main. Normal desktop CI verifies and downloads this exact release; merge through the ordinary review process."
if [[ -n $number ]]; then
  gh pr edit "$number" --repo "$repo" --title "$title" --body "$body"
else
  gh pr create --repo "$repo" --base main --head "$branch" --title "$title" --body "$body"
fi
