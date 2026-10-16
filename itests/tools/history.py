"""Write results/index.html: every test run, its totals and what changed since the previous run.

Usage: uv run --no-project python tools/history.py RESULTS_DIR
"""

import html
import json
import sys
from pathlib import Path


def load(run: Path) -> dict | None:
    try:
        report = json.loads((run / "report.json").read_text())
    except (OSError, ValueError):
        return None
    args = (run / "args.txt").read_text().strip() or "всё" if (run / "args.txt").exists() else "—"
    tests = {t["id"]: t["status"] for t in report["tests"]}
    return {"dir": run.name, "args": args, "report": report, "tests": tests}


def changes(cur: dict, prev: dict | None) -> tuple[list[str], list[str]]:
    """Tests failing now but passing before, and the reverse; only tests present in both runs."""
    if prev is None:
        return [], []
    common = cur["tests"].keys() & prev["tests"].keys()
    broke = sorted(t for t in common if cur["tests"][t] != "PASS" and prev["tests"][t] == "PASS")
    fixed = sorted(t for t in common if cur["tests"][t] == "PASS" and prev["tests"][t] != "PASS")
    return broke, fixed


def ids(items: list[str], cls: str) -> str:
    if not items:
        return ""
    shown = "".join(f"<li>{html.escape(i)}</li>" for i in items[:30])
    more = f"<li>… +{len(items) - 30}</li>" if len(items) > 30 else ""
    return f'<details class="{cls}"><summary>{len(items)}</summary><ul>{shown}{more}</ul></details>'


def main(results: Path):
    runs = [r for r in (load(p) for p in sorted(results.iterdir()) if p.is_dir() and p.name[:1].isdigit()) if r]
    rows = []
    for i, run in enumerate(runs):
        # Compare with the previous run of the same selection (args), else the previous run.
        prev = next((p for p in reversed(runs[:i]) if p["args"] == run["args"]), runs[i - 1] if i else None)
        broke, fixed = changes(run, prev)
        c = run["report"]["counts"]
        total = sum(c.values())
        failed = c["FAIL"] + c["BLOCKED"]
        report = f'<a href="{run["dir"]}/report.html">отчёт</a>' if (results / run["dir"] / "report.html").exists() else ""
        rows.append(f"""<tr class="{'bad' if failed else 'good'}">
<td class="when">{html.escape(run['report']['started'][:19].replace('T', ' '))} UTC</td>
<td><code>{html.escape(run['args'])}</code></td>
<td class="num">{total}</td><td class="num pass">{c['PASS']}</td>
<td class="num fail">{c['FAIL'] or ''}</td><td class="num blocked">{c['BLOCKED'] or ''}</td>
<td class="num">{run['report']['seconds']:.0f}s</td>
<td>{ids(broke, 'broke')}</td><td>{ids(fixed, 'fixed')}</td>
<td class="links">{report} <a href="{run['dir']}/summary.txt">summary</a> <a href="{run['dir']}/logs/">логи</a></td>
</tr>""")
    page = f"""<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>tilr-itests runs</title>
<style>
:root {{ --bg:#fff; --fg:#1d2433; --muted:#6b7280; --line:#e5e7eb; --pass:#15803d; --fail:#b91c1c; --blocked:#b45309; --row:#fef2f2; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#111827; --fg:#e5e7eb; --muted:#9ca3af; --line:#374151;
  --pass:#4ade80; --fail:#f87171; --blocked:#fbbf24; --row:#2a1618; }} }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; margin:0; padding:24px 16px; }}
h1 {{ font-size:20px; margin:0 0 4px; }} p {{ color:var(--muted); margin:0 0 16px; }}
.wrap {{ overflow-x:auto; }}
table {{ border-collapse:collapse; width:100%; }}
th, td {{ border-bottom:1px solid var(--line); padding:6px 10px; text-align:left; vertical-align:top; white-space:nowrap; }}
th {{ color:var(--muted); font-weight:600; }}
tr.bad td:first-child {{ box-shadow:inset 3px 0 var(--fail); }} tr.good td:first-child {{ box-shadow:inset 3px 0 var(--pass); }}
.num {{ text-align:right; font-variant-numeric:tabular-nums; }}
.pass {{ color:var(--pass); }} .fail {{ color:var(--fail); font-weight:600; }} .blocked {{ color:var(--blocked); }}
details.broke summary {{ color:var(--fail); cursor:pointer; }} details.fixed summary {{ color:var(--pass); cursor:pointer; }}
details ul {{ margin:4px 0 0; padding-left:18px; white-space:normal; }}
a {{ color:inherit; }} .links a {{ margin-right:8px; }}
</style></head><body>
<h1>tilr-itests — прогоны</h1>
<p>{len(runs)} прогонов, новые сверху. «Сломалось / починилось» — по сравнению с предыдущим прогоном с теми же аргументами.</p>
<div class="wrap"><table>
<tr><th>Когда</th><th>Аргументы</th><th class="num">Всего</th><th class="num">PASS</th><th class="num">FAIL</th>
<th class="num">BLOCKED</th><th class="num">Время</th><th>Сломалось</th><th>Починилось</th><th></th></tr>
{''.join(reversed(rows))}
</table></div></body></html>
"""
    (results / "index.html").write_text(page)


if __name__ == "__main__":
    main(Path(sys.argv[1]))
