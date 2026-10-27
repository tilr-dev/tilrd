"""Regression tests for preserving the recorded itests run in coverage output."""

import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "coverage.py"
SPEC = importlib.util.spec_from_file_location("itest_coverage_tool", SCRIPT)
coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(coverage)


class NamedTestsTest(unittest.TestCase):
    def test_python_citations_cover_requirements_without_becoming_cargo_tests(self):
        citations = ["itests/tests/test_coverage.py::CoverageRunTest",
                     "scripts/test_server_cleanup_stand_selftest.py"]
        text = "**Verified by**: " + " and ".join(f"`{name}`" for name in citations + citations) + "."
        specs = {"sample": {"requirements": {"example": {
            "title": "Python evidence", "text": text, "scenarios": {},
        }}}}

        result = coverage.build({"cases": []}, specs, {}, {})

        requirement = result["capabilities"][0]["requirements"][0]
        self.assertTrue(requirement["covered"])
        self.assertEqual(requirement["python_tests"], citations)
        self.assertEqual(requirement["cargo_tests"], [])
        markdown = coverage.markdown(result)
        self.assertIn("Python tests named in the spec:", markdown)
        for name in citations:
            self.assertIn(f"`{name}`", markdown)
        self.assertNotIn("Cargo tests named in the spec:", markdown)

    def test_cargo_citations_keep_crate_qualified_names(self):
        for label in ("Verified by", "Verified by:", "**Verified by**:", "**Verified by:**"):
            with self.subTest(label=label):
                text = label + " `tilr/build_script::watched_files` and `module::tests::works`."
                self.assertEqual(coverage.named_tests(text),
                                 ["tilr/build_script::watched_files", "module::tests::works"])

    def test_unrelated_paths_and_purpose_do_not_cover_a_requirement(self):
        specs = {"sample": {"purpose": "Verified by `itests/tests/test_coverage.py`.",
                            "requirements": {"example": {
                                "title": "No evidence", "scenarios": {},
                                "text": "See `itests/tests/test_coverage.py`. Verified by "
                                        "`itests/tools/coverage.py`, `scripts/helper.py`, "
                                        "`itests/scenarios/uc_007.py`, `uc_007::case`.",
                            }}}}
        requirement = coverage.build({"cases": []}, specs, {}, {})["capabilities"][0]["requirements"][0]
        self.assertFalse(requirement["covered"])
        self.assertEqual(requirement["python_tests"], [])
        self.assertEqual(requirement["cargo_tests"], [])


class CoverageRunTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.out = self.root / "coverage"
        self.specs = {"sample": {"requirements": {"example": {
            "title": "Current requirement", "text": "Current requirement text",
            "scenarios": {"current": "Current scenario"},
        }}}}
        self.registry = {"cases": [
            {"id": "case-a", "spec": ["sample/example#current"]},
            {"id": "case-new", "spec": ["sample/example#current"]},
        ]}

    def generate(self, *args):
        with (mock.patch.object(coverage, "REPO", self.root),
              mock.patch.object(coverage, "ITESTS", self.root / "itests"),
              mock.patch.object(coverage, "load_registry", return_value=self.registry),
              mock.patch.object(coverage.specmod, "load", return_value=self.specs),
              mock.patch.object(sys, "argv", [str(SCRIPT), "--out", str(self.out), *args]),
              contextlib.redirect_stdout(io.StringIO())):
            return coverage.main()

    def output(self):
        return json.loads((self.out / "coverage.json").read_text())

    def test_checkout_generates_local_reports_ignored_by_git(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--out", str(self.out)],
                                cwd=SCRIPT.parents[2], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.output()["capabilities"])
        self.assertTrue((self.out / "coverage.md").read_text().startswith("# Spec coverage"))
        paths = ["openspec/coverage/coverage.json", "openspec/coverage/coverage.md"]
        tracked = subprocess.run(["git", "ls-files", "openspec/coverage"],
                                 cwd=SCRIPT.parents[2], check=True, capture_output=True, text=True)
        self.assertEqual(tracked.stdout, "")
        ignored = subprocess.run(["git", "check-ignore", *paths], cwd=SCRIPT.parents[2],
                                 capture_output=True, text=True)
        self.assertEqual(ignored.returncode, 0, ignored.stderr)
        self.assertEqual(ignored.stdout.splitlines(), paths)

    def test_missing_local_report_preserves_previous_run(self):
        self.out.mkdir()
        previous = {"generated_from": {"last_run": {
            "started": "2026-10-18T10:00:00Z", "commit": "old", "counts": {"PASS": 1},
            "args": "case case-a", "path": "itests/results/old",
        }}, "cases": [{"id": "case-a", "last_run": "PASS"}]}
        (self.out / "coverage.json").write_text(json.dumps(previous))

        self.assertEqual(self.generate(), 0)

        current = self.output()
        self.assertEqual(current["generated_from"]["last_run"], previous["generated_from"]["last_run"])
        self.assertEqual({case["id"]: case["last_run"] for case in current["cases"]},
                         {"case-a": "PASS", "case-new": "NOT_RUN"})
        self.assertEqual(current["capabilities"][0]["requirements"][0]["title"], "Current requirement")
        markdown = (self.out / "coverage.md").read_text()
        self.assertIn("- Last run: 2026-10-18T10:00:00 UTC", markdown)
        self.assertIn("`case-a` (PASS)", markdown)

    def test_explicit_report_supplies_run_and_statuses(self):
        local = self.root / "itests" / "results" / "latest" / "report.json"
        local.parent.mkdir(parents=True)
        local.write_text(json.dumps({
            "started": "2026-10-18T12:00:00Z", "commit": "local", "counts": {"PASS": 1},
            "tests": [{"kind": "scenario", "id": "scenario:case-a", "status": "PASS"}],
        }))
        report = self.root / "other-worktree" / "report.json"
        report.parent.mkdir()
        report.write_text(json.dumps({
            "started": "2026-10-19T12:00:00Z", "commit": "new", "counts": {"FAIL": 1},
            "tests": [{"kind": "scenario", "id": "scenario:case-a", "status": "FAIL"}],
        }))

        self.assertEqual(self.generate("--report", str(report)), 0)

        current = self.output()
        self.assertEqual(current["generated_from"]["last_run"]["commit"], "new")
        self.assertEqual(current["cases"][0]["last_run"], "FAIL")
        self.assertIn("`case-a` (FAIL)", (self.out / "coverage.md").read_text())

    def test_local_report_supplies_run_when_no_explicit_path(self):
        report = self.root / "itests" / "results" / "latest" / "report.json"
        report.parent.mkdir(parents=True)
        report.write_text(json.dumps({
            "started": "2026-10-19T11:00:00Z", "commit": "local", "counts": {"PASS": 1},
            "tests": [{"kind": "scenario", "id": "scenario:case-a", "status": "PASS"}],
        }))

        self.assertEqual(self.generate(), 0)
        self.assertEqual(self.output()["generated_from"]["last_run"]["commit"], "local")
        self.assertEqual(self.output()["cases"][0]["last_run"], "PASS")

    def test_no_report_or_previous_coverage_has_no_last_run(self):
        self.assertEqual(self.generate(), 0)
        self.assertIsNone(self.output()["generated_from"]["last_run"])
        self.assertTrue(all(case["last_run"] is None for case in self.output()["cases"]))
        self.assertIn("- Last run: none", (self.out / "coverage.md").read_text())

    def test_missing_explicit_report_does_not_replace_output(self):
        self.out.mkdir()
        old_json = self.out / "coverage.json"
        old_md = self.out / "coverage.md"
        old_json.write_text("old json")
        old_md.write_text("old markdown")

        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as error:
                self.generate("--report", str(self.root / "missing.json"))

        self.assertNotEqual(error.exception.code, 0)
        self.assertEqual(old_json.read_text(), "old json")
        self.assertEqual(old_md.read_text(), "old markdown")

    def test_unreadable_previous_coverage_fails_without_writing(self):
        self.out.mkdir()
        old_json = self.out / "coverage.json"
        old_md = self.out / "coverage.md"
        old_md.write_text("old markdown")
        for name, content in {"truncated": '{"cases": [', "missing keys": "{}", "wrong type": "[1]",
                              "bad case": '{"cases": [1], "generated_from": {}}'}.items():
            with self.subTest(name):
                old_json.write_text(content)
                stderr = io.StringIO()
                with contextlib.redirect_stderr(stderr):
                    with self.assertRaises(SystemExit) as error:
                        self.generate()

                self.assertEqual(error.exception.code, 2)
                self.assertIn(f"cannot read previous coverage.json at {old_json}", stderr.getvalue())
                self.assertEqual(old_json.read_text(), content)
                self.assertEqual(old_md.read_text(), "old markdown")


if __name__ == "__main__":
    unittest.main()
