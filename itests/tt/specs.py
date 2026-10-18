"""Parse openspec/specs/<capability>/spec.md into requirement and scenario slugs.

A tag is CAPABILITY/REQUIREMENT-SLUG[#SCENARIO-SLUG]; a slug is the heading text in
lowercase with every run of non-alphanumerics collapsed to "-" (no leading/trailing "-").
"none:REASON" is accepted for harness self-checks. Stdlib only: it runs on the host
(`./dt coverage`) and in `python3 -m tt list`, where the specs are found only in a checkout.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

REQ = re.compile(r"^###\s+Requirement:\s*(.+?)\s*$")
SCEN = re.compile(r"^####\s+Scenario:\s*(.+?)\s*$")


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")


def specs_dir() -> Path:
    env = os.environ.get("TT_SPECS")
    if env:
        return Path(env)
    return Path(__file__).resolve().parents[2] / "openspec" / "specs"


def load(root: Path | None = None) -> dict:
    """{capability: {"path": str, "requirements": {req_slug: {"title", "text", "scenarios": {slug: title}}}}}"""
    root = root or specs_dir()
    out: dict = {}
    if not root.is_dir():
        raise FileNotFoundError(f"specs directory not found: {root}")
    for spec in sorted(root.glob("*/spec.md")):
        cap = spec.parent.name
        reqs: dict = {}
        current = None
        for line in spec.read_text().splitlines():
            m = REQ.match(line)
            if m:
                current = {"title": m.group(1), "text": "", "scenarios": {}}
                reqs[slug(m.group(1))] = current
                continue
            if line.startswith("## ") or line.startswith("### "):
                current = None
                continue
            m = SCEN.match(line)
            if m and current is not None:
                current["scenarios"][slug(m.group(1))] = m.group(1)
                continue
            if current is not None:
                current["text"] += line + "\n"
        out[cap] = {"path": str(spec), "requirements": reqs}
    return out


def check_tag(tag: str, specs: dict) -> str | None:
    """None when the tag resolves, else the reason."""
    if tag.startswith("none:"):
        return None if tag[5:].strip() else "none: needs a reason"
    cap, sep, rest = tag.partition("/")
    if not sep or not rest:
        return "expected CAPABILITY/REQUIREMENT[#SCENARIO]"
    if cap not in specs:
        return f"unknown capability {cap}"
    req, _, scen = rest.partition("#")
    reqs = specs[cap]["requirements"]
    if req not in reqs:
        return f"no requirement {req} in {cap}"
    if scen and scen not in reqs[req]["scenarios"]:
        return f"no scenario {scen} in {cap}/{req}"
    return None
