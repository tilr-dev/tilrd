"""Write openspec/coverage/coverage.md and coverage.json: spec requirements and scenarios
mapped to the Dagger cases tagged for them, the cargo tests the spec text names, and the
status of each case in the last run (from --report, the local latest report, or
the existing coverage.json).

Runs on the host without Dagger (stdlib only):

    uv run --no-project python itests/tools/coverage.py [--out DIR] [--report PATH]

Exits 1 when the registry is invalid (an untagged case, a tag naming no requirement or
scenario): the same validation `python3 -m tt list` does in a checkout.

A case is tagged with the requirements it verifies, `@case(spec=["<capability>/<requirement
slug>[#<scenario slug>]"])`; the tags follow the spec's "**Verified by**" lines, which also
name the cargo tests. A requirement is covered by a tagged case or by a cargo test the spec
names; whether that test still exists is not checked here.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import re
import sys
from pathlib import Path

ITESTS = Path(__file__).resolve().parents[1]
REPO = ITESTS.parent
sys.path.insert(0, str(ITESTS))

from tt import specs as specmod  # noqa: E402

# "Verified by `a::b` and `c`", "covered by `x`": the cargo tests a requirement names
# (`file_stem::test`, unit tests `module::tests::test`; itests citations are skipped).
NAMED = re.compile(r"(?:[Vv]erified|[Cc]overed|[Tt]ested) by(?:\*\*:)? ([^.]*(?:\.(?!\s)[^.]*)*)")
TICKED = re.compile(r"`([A-Za-z_][\w:]*(?:::[\w]+)*)`")


def named_tests(text: str) -> list[str]:
    out: list[str] = []
    for m in NAMED.finditer(text):
        for name in TICKED.findall(m.group(1)):
            if name not in out and ("::" in name or "_" in name) and not name.startswith("uc_"):
                out.append(name)
    return out


def load_registry() -> dict:
    import tt.__main__ as reg  # noqa: PLC0415
    err = io.StringIO()
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            return reg.registry()
    except SystemExit as e:
        print(e.code if isinstance(e.code, str) else f"registry failed ({e.code})", file=sys.stderr)
        print(err.getvalue(), file=sys.stderr, end="")
        sys.exit(1)


class PreviousCoverageError(Exception):
    """The fallback coverage.json exists but cannot be read."""


def last_run(path: Path, coverage_path: Path) -> tuple[dict, dict]:
    if not path.exists():
        if not coverage_path.exists():
            return {}, {}
        try:
            previous = json.loads(coverage_path.read_text())
            status = {case["id"]: case["last_run"] for case in previous["cases"] if case.get("last_run")}
            return status, previous["generated_from"].get("last_run") or {}
        except (OSError, json.JSONDecodeError, KeyError, TypeError, AttributeError) as e:
            reason = f"missing key {e}" if isinstance(e, KeyError) else str(e)
            raise PreviousCoverageError(f"cannot read previous coverage.json at {coverage_path}: {reason}; "
                                        "delete it or pass --report PATH") from e
    report = json.loads(path.read_text())
    status: dict[str, str] = {}
    for t in report["tests"]:
        if t["kind"] == "scenario":
            # With --repeat a case runs more than once: the worst outcome counts.
            case_id = t["id"].removeprefix("scenario:")
            if status.get(case_id) in (None, "PASS"):
                status[case_id] = t["status"]
    args = path.parent / "args.txt"
    meta = {"started": report.get("started"), "commit": report.get("commit"), "counts": report.get("counts"),
            "args": args.read_text().strip() if args.exists() else None,
            "path": str(path.resolve().parent.relative_to(REPO)) if path.resolve().is_relative_to(REPO) else str(path)}
    return status, meta


def build(registry: dict, specs: dict, status: dict, meta: dict) -> dict:
    by_tag: dict[str, list[str]] = {}
    for c in registry["cases"]:
        for tag in c["spec"]:
            by_tag.setdefault(tag, []).append(c["id"])
    caps = []
    for cap, spec in specs.items():
        reqs = []
        for rslug, req in spec["requirements"].items():
            base = f"{cap}/{rslug}"
            req_cases = sorted(by_tag.get(base, []))
            scenarios = []
            for sslug, title in req["scenarios"].items():
                cases = sorted(by_tag.get(f"{base}#{sslug}", []))
                scenarios.append({"slug": sslug, "title": title, "tag": f"{base}#{sslug}", "cases": cases,
                                  "covered": bool(cases)})
            all_cases = sorted(set(req_cases) | {c for s in scenarios for c in s["cases"]})
            tests = named_tests(req["text"])
            reqs.append({"slug": rslug, "title": req["title"], "tag": base, "cases": req_cases,
                         "all_cases": all_cases, "cargo_tests": tests, "scenarios": scenarios,
                         "covered": bool(all_cases or tests),
                         "last_run": {c: status.get(c, "NOT_RUN") for c in all_cases} if status else {}})
        caps.append({"capability": cap, "requirements": reqs})
    none_tags = sorted({t for c in registry["cases"] for t in c["spec"] if t.startswith("none:")})
    return {
        "generated_from": {"specs": "openspec/specs", "cases": len(registry["cases"]), "last_run": meta or None},
        "capabilities": caps,
        "cases": [{"id": c["id"], "spec": c["spec"],
                   "last_run": status.get(c["id"], "NOT_RUN") if status else None} for c in registry["cases"]],
        "untagged_by_spec": none_tags,
    }


def markdown(data: dict) -> str:
    caps = data["capabilities"]
    reqs = [r for c in caps for r in c["requirements"]]
    covered = sum(r["covered"] for r in reqs)
    by_dagger = sum(bool(r["all_cases"]) for r in reqs)
    scen = [s for r in reqs for s in r["scenarios"]]
    meta = data["generated_from"]["last_run"]
    out = ["# Spec coverage", "",
           "Generated by `./itests/dt coverage` (itests/tools/coverage.py) from `openspec/specs/` and the case "
           "tags of `itests/scenarios/`. Do not edit by hand.", "",
           f"- Requirements: {len(reqs)}; covered {covered} (Dagger cases: {by_dagger}; cargo tests named in "
           f"the spec only: {covered - by_dagger}); uncovered {len(reqs) - covered}",
           f"- Scenarios with a tagged case: {sum(s['covered'] for s in scen)} of {len(scen)}",
           f"- Cases: {data['generated_from']['cases']}"]
    if meta:
        c = meta.get("counts") or {}
        run = f"- Last run: {(meta.get('started') or '')[:19]} UTC"
        if meta.get("commit"):
            run += f", commit {meta['commit']}"
        if meta.get("args"):
            run += f", `./dt {meta['args']}`"
        out.append(run + ", " + ", ".join(f"{k} {v}" for k, v in c.items() if v))
    else:
        out.append("- Last run: none (no itests/results/latest/report.json)")
    out += ["", "## Summary", "", "| Capability | Requirements | Covered | Dagger | Uncovered |", "|---|---|---|---|---|"]
    for c in caps:
        rs = c["requirements"]
        out.append(f"| {c['capability']} | {len(rs)} | {sum(r['covered'] for r in rs)} | "
                   f"{sum(bool(r['all_cases']) for r in rs)} | {sum(not r['covered'] for r in rs)} |")
    out += ["", "## Uncovered", ""]
    unc = [(c["capability"], r) for c in caps for r in c["requirements"] if not r["covered"]]
    out += [f"- `{cap}/{r['slug']}` — {r['title']}" for cap, r in unc] or ["None."]
    for c in caps:
        out += ["", f"## {c['capability']}", ""]
        for r in c["requirements"]:
            mark = "covered" if r["covered"] else "**uncovered**"
            out.append(f"### {r['title']} ({mark})")
            out.append("")
            out.append(f"Tag `{r['tag']}`.")
            if r["cargo_tests"]:
                out.append("Cargo tests named in the spec: " + ", ".join(f"`{t}`" for t in r["cargo_tests"]) + ".")
            if r["cases"]:
                out.append("Cases (requirement): " + ", ".join(_case(r, x) for x in r["cases"]) + ".")
            for s in r["scenarios"]:
                cases = ", ".join(_case(r, x) for x in s["cases"]) or "—"
                out.append(f"- Scenario *{s['title']}*: {cases}")
            out.append("")
    return "\n".join(out).rstrip() + "\n"


def _case(req: dict, case_id: str) -> str:
    st = req["last_run"].get(case_id) if req["last_run"] else None
    return f"`{case_id}`" + (f" ({st})" if st else "")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, default=REPO / "openspec" / "coverage")
    ap.add_argument("--report", type=Path, help="read run statuses from this report.json")
    args = ap.parse_args()
    if args.report is not None and not args.report.is_file():
        ap.error(f"report not found: {args.report}")
    specs = specmod.load(REPO / "openspec" / "specs")
    registry = load_registry()
    try:
        status, meta = last_run(args.report or ITESTS / "results" / "latest" / "report.json",
                                args.out / "coverage.json")
    except PreviousCoverageError as e:
        ap.error(str(e))
    data = build(registry, specs, status, meta)
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "coverage.json").write_text(json.dumps(data, indent=1) + "\n")
    (args.out / "coverage.md").write_text(markdown(data))
    reqs = [r for c in data["capabilities"] for r in c["requirements"]]
    print(f"coverage: {sum(r['covered'] for r in reqs)}/{len(reqs)} requirements covered; "
          f"wrote {args.out / 'coverage.md'} and coverage.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
