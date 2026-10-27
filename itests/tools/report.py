"""Write RUN/report.html from RUN/report.json and RUN/logs/: one self-contained page per run.

Usage: uv run --no-project python tools/report.py RUN_DIR
"""

import html
import json
import sys
from pathlib import Path

PASS_LOG = 8000      # tail kept inline for passing tests; the full log is linked
FAIL_LOG = 60000


def group(test: dict) -> str:
    kind, _, rest = test["id"].partition(":")
    if kind == "rust":
        return "rust " + rest.split("::")[0]
    if kind == "scenario":
        return "scenario " + rest.split(":")[0]
    return kind


def main(run: Path):
    report = json.loads((run / "report.json").read_text())
    args = (run / "args.txt").read_text().strip() if (run / "args.txt").exists() else ""
    tests = report["tests"]
    order = {"FAIL": 0, "BLOCKED": 1, "PASS": 2}
    tests.sort(key=lambda t: (order[t["status"]], t["id"]))
    kinds = sorted({t["kind"] for t in tests})
    rows = []
    for t in tests:
        path = run / t["log"]
        log = path.read_text(errors="replace") if path.exists() else ""
        limit = PASS_LOG if t["status"] == "PASS" else FAIL_LOG
        cut = f"… {len(log) - limit} chars above, full log: {t['log']}\n" if len(log) > limit else ""
        open_ = " open" if t["status"] != "PASS" else ""
        rows.append(
            f'<details class="t {t["status"].lower()}" data-status="{t["status"]}" data-kind="{t["kind"]}"{open_}>'
            f'<summary><span class="st">{t["status"]}</span><span class="g">{html.escape(group(t))}</span>'
            f'<span class="id">{html.escape(t["id"])}</span><span class="ms">{t["ms"] / 1000:.1f}s</span></summary>'
            f'<pre>{html.escape(cut + log[-limit:])}</pre><a href="{html.escape(t["log"])}">полный лог</a></details>')
    c = report["counts"]
    buttons = "".join(f'<label><input type="checkbox" data-filter="status" value="{s}" checked> {s} '
                      f'<b>{c[s]}</b></label>' for s in ("FAIL", "BLOCKED", "PASS"))
    kind_buttons = "".join(f'<label><input type="checkbox" data-filter="kind" value="{k}" checked> {k} '
                           f'<b>{sum(t["kind"] == k for t in tests)}</b></label>' for k in kinds)
    page = f"""<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>tilr-itests {html.escape(run.name)}</title>
<style>
:root {{ --bg:#fff; --fg:#1d2433; --muted:#6b7280; --line:#e5e7eb; --code:#f6f8fa;
  --pass:#15803d; --fail:#b91c1c; --blocked:#b45309; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#111827; --fg:#e5e7eb; --muted:#9ca3af; --line:#374151;
  --code:#0b1220; --pass:#4ade80; --fail:#f87171; --blocked:#fbbf24; }} }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; margin:0; padding:24px 16px; }}
h1 {{ font-size:20px; margin:0 0 4px; }} .meta {{ color:var(--muted); margin:0 0 14px; }}
.bar {{ display:flex; flex-wrap:wrap; gap:6px 16px; align-items:center; margin:0 0 14px; position:sticky; top:0;
  background:var(--bg); padding:8px 0; border-bottom:1px solid var(--line); }}
.bar input[type=search] {{ flex:1 1 220px; padding:6px 8px; border:1px solid var(--line); border-radius:6px;
  background:var(--bg); color:var(--fg); }}
.bar label {{ white-space:nowrap; cursor:pointer; }}
details.t {{ border-bottom:1px solid var(--line); }}
details.t summary {{ display:flex; gap:12px; padding:6px 4px; cursor:pointer; align-items:baseline; }}
.st {{ font-weight:700; width:64px; flex:none; }} .pass .st {{ color:var(--pass); }}
.fail .st {{ color:var(--fail); }} .blocked .st {{ color:var(--blocked); }}
.g {{ color:var(--muted); width:260px; flex:none; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
.id {{ flex:1; overflow-wrap:anywhere; }} .ms {{ color:var(--muted); font-variant-numeric:tabular-nums; }}
pre {{ background:var(--code); padding:10px; overflow-x:auto; font-size:12px; margin:4px 0; max-height:480px; }}
details.t a {{ color:var(--muted); font-size:12px; display:inline-block; margin:0 0 8px 4px; }}
.empty {{ color:var(--muted); padding:20px 0; display:none; }}
@media (max-width:640px) {{ .g {{ display:none; }} }}
</style></head><body>
<h1>tilr-itests — {html.escape(run.name)}</h1>
<p class="meta">{html.escape(report['started'][:19].replace('T', ' '))} UTC · {report['seconds']:.0f}s ·
аргументы: <code>{html.escape(args or 'всё')}</code> · <a href="../index.html">история</a> · <a href="summary.txt">summary</a></p>
<div class="bar"><input type="search" placeholder="поиск по имени теста"> {buttons}<span>·</span>{kind_buttons}</div>
<div id="list">{''.join(rows)}</div>
<p class="empty">Нет тестов под фильтр.</p>
<script>
const q = document.querySelector('input[type=search]');
const boxes = [...document.querySelectorAll('input[data-filter]')];
function apply() {{
  const on = f => new Set(boxes.filter(b => b.dataset.filter === f && b.checked).map(b => b.value));
  const st = on('status'), kd = on('kind'), text = q.value.toLowerCase();
  let shown = 0;
  for (const d of document.querySelectorAll('details.t')) {{
    const ok = st.has(d.dataset.status) && kd.has(d.dataset.kind) && d.querySelector('.id').textContent.toLowerCase().includes(text);
    d.style.display = ok ? '' : 'none'; shown += ok;
  }}
  document.querySelector('.empty').style.display = shown ? 'none' : 'block';
}}
q.addEventListener('input', apply); boxes.forEach(b => b.addEventListener('change', apply));
</script></body></html>
"""
    (run / "report.html").write_text(page)


if __name__ == "__main__":
    main(Path(sys.argv[1]))
