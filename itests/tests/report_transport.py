"""Exercise the real report through Dagger, without building or running the suite.

Run: python3 itests/tests/report_transport.py [--source path/to/main.py]
Requires the pinned Dagger CLI and an engine. The source option permits checking
the same regression against the implementation before the fix.
"""

import argparse
import ast
import json
import os
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET


SOURCE = Path(__file__).resolve().parents[1] / "src/tilr_itests/main.py"
MARKER = "report-transport-payload"


def log_for(index):
    line = f'{MARKER} {index}: Unicode café 雪 <xml> & "quoted" \\ slash\n'
    return (line * (400_000 // len(line) + 1))[:400_000]


def check(source):
    tree = ast.parse(source.read_text())
    selected = []
    for node in tree.body:
        if getattr(node, "name", "") in ("Result", "_safe", "_report"):
            selected.append(ast.unparse(node))
        elif isinstance(node, ast.Assign) and any(
                isinstance(t, ast.Name) and t.id == "UNBUNDLE" for t in node.targets):
            selected.append(ast.unparse(node))
    report = next(n for n in tree.body if getattr(n, "name", "") == "_report")
    # Adapt the harness to the original signature, leaving the report unchanged.
    call = "_report(results, started)" if len(report.args.args) == 2 else (
        '_report(dag.container().from_("python:3.14-alpine"), results, started)')
    if isinstance(report, ast.AsyncFunctionDef):
        call = "await " + call
    module = "\n".join([
        "import dataclasses, datetime, json, os, tempfile",
        "from pathlib import Path",
        "import dagger",
        "from dagger import dag, function, object_type",
        *selected,
        ast.unparse(next(n for n in ast.parse(Path(__file__).read_text()).body
                         if getattr(n, "name", "") == "log_for")),
        f"MARKER = {MARKER!r}",
        "@object_type\nclass ReportTransport:",
        "    @function\n    async def report(self) -> dagger.Directory:",
        "        started = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)",
        "        results = [Result(f'rust:probe::{i}', 'rust',",
        "                          'FAIL' if i == 0 else 'BLOCKED' if i == 1 else 'PASS',",
        "                          i, log_for(i)) for i in range(710)]",
        f"        return {call}",
    ])
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        (root / "src/report_transport").mkdir(parents=True)
        (root / "src/report_transport/__init__.py").write_text("from .main import ReportTransport\n")
        (root / "src/report_transport/main.py").write_text(module)
        (root / "dagger.json").write_text(json.dumps({
            "name": "report-transport", "engineVersion": "v0.21.10",
            "sdk": {"source": "python"},
        }))
        (root / "pyproject.toml").write_text(
            '[project]\nname = "report-transport"\nversion = "0.1.0"\n'
            'requires-python = ">=3.14"\ndependencies = ["dagger-io"]\n'
            '[build-system]\nrequires = ["uv_build>=0.8.4,<0.9.0"]\n'
            'build-backend = "uv_build"\n'
            '[tool.uv.sources]\ndagger-io = { path = "sdk", editable = true }\n')
        output = root / "report"
        progress = root / "progress.log"
        with progress.open("wb") as stream:
            result = subprocess.run([
                os.environ.get("DT_ENGINE", "dagger"), "-m", str(root),
                "--progress=plain", "call", "report", "export", "--path", str(output),
            ], stdout=stream, stderr=subprocess.STDOUT, env=dict(os.environ, DAGGER_NO_NAG="1"))
        # The payload itself must never enter GraphQL span attributes: this is the
        # source of telemetry amplification, independently of engine memory/speed.
        with progress.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                assert MARKER.encode() not in chunk, "report payload leaked into Dagger telemetry"
        assert result.returncode == 0, progress.read_text()[-6000:]
        counts = {"PASS": 708, "FAIL": 1, "BLOCKED": 1}
        data = json.loads((output / "report.json").read_text())
        assert data["counts"] == counts
        assert len(data["tests"]) == 710
        for index in range(710):
            assert (output / f"logs/rust_probe__{index}.log").read_text() == log_for(index)
        summary = (output / "summary.txt").read_text()
        assert "TOTAL 710: 708 passed, 1 failed, 1 blocked" in summary
        assert (output / "status").read_text() == "failed\n"
        junit = ET.parse(output / "junit.xml").getroot()
        assert (junit.attrib["tests"], junit.attrib["failures"], junit.attrib["errors"]) == ("710", "1", "1")
        print("report transport: 710 complete logs, counts, summary, JUnit and status verified; no payload in telemetry")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    check(parser.parse_args().source)
