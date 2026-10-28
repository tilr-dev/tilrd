"""Deterministic integration cases for the release branch pull request gate using real Git repos."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
from release_upstream import check, git


class ReleaseUpstream(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        git(self.repo, "init", "-b", "main")
        git(self.repo, "config", "user.name", "Fixture")
        git(self.repo, "config", "user.email", "fixture@example.test")
        git(self.repo, "config", "commit.gpgsign", "false")
        self.commit("base", "base")
        git(self.repo, "branch", "release/0.3")
        self.event = {"repository": {"full_name": "org/repo"}, "pull_request": {
            "base": {"ref": "release/0.3"},
            "head": {"ref": "fix", "repo": {"full_name": "org/repo"}},
            "user": {"login": "contributor", "type": "User"},
        }}

    def commit(self, name, content, message="fix: fixture"):
        (self.repo / name).write_text(content)
        git(self.repo, "add", name)
        git(self.repo, "commit", "-m", message)
        return git(self.repo, "rev-parse", "HEAD")

    def run_gate(self, success):
        pr = self.event["pull_request"]
        for name, revision in [("base", "release/0.3"), ("head", "HEAD"), ("main", "main")]:
            sha = git(self.repo, "rev-parse", revision)
            git(self.repo, "update-ref", f"refs/inspection/{name}", sha)
            if name != "main":
                pr[name]["sha"] = sha
        event = self.repo / "event.json"
        event.write_text(json.dumps(self.event))
        result = subprocess.run([sys.executable, str(Path(__file__).with_name("release_upstream.py")),
                                 "--repo", str(self.repo), "--event", str(event)],
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0 if success else 1, result.stdout + result.stderr)
        if not success:
            self.assertIn("Land each change in main first", result.stderr)
            self.assertIn(pr["head"]["sha"], result.stderr)

    def cherry_pick(self):
        source = self.commit("fix", "upstream")
        git(self.repo, "checkout", "release/0.3")
        # Different parent ensures a different SHA without relying on wall-clock time.
        self.commit("release-only", "release metadata")
        git(self.repo, "cherry-pick", source)
        self.assertNotEqual(source, git(self.repo, "rev-parse", "HEAD"))
        git(self.repo, "checkout", "-b", "candidate")
        # The release-only commit belongs to the base, not to the PR.
        git(self.repo, "branch", "-f", "release/0.3", "HEAD~1")

    def test_direct_fix_and_forged_trailer_fail(self):
        source = git(self.repo, "rev-parse", "main")
        git(self.repo, "checkout", "-b", "candidate", "release/0.3")
        self.commit("novel", "new", f"fix: forged\n\n(cherry picked from commit {source})")
        self.run_gate(False)

    def test_cherry_pick_without_trailer_passes(self):
        self.cherry_pick()
        self.run_gate(True)

    def test_mixed_pr_fails(self):
        self.cherry_pick()
        self.commit("novel", "new")
        self.run_gate(False)

    def test_commit_in_main_passes(self):
        self.commit("fix", "upstream")
        self.run_gate(True)

    def test_missing_history_and_stale_event_fail_closed(self):
        self.commit("fix", "upstream")
        self.run_gate(True)
        self.event["pull_request"]["head"]["sha"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "does not match"):
            check(self.repo, self.event)
        self.event["pull_request"]["head"]["sha"] = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "update-ref", "-d", "refs/inspection/main")
        with self.assertRaises(subprocess.CalledProcessError):
            check(self.repo, self.event)

    def test_binary_cherry_pick_passes(self):
        (self.repo / "binary").write_bytes(b"\x00\x01\x02")
        git(self.repo, "add", "binary")
        git(self.repo, "commit", "-m", "fix: binary")
        source = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-b", "candidate", "release/0.3")
        self.commit("release-only", "release metadata")
        git(self.repo, "branch", "-f", "release/0.3", "HEAD")
        git(self.repo, "cherry-pick", source)
        self.assertNotEqual(source, git(self.repo, "rev-parse", "HEAD"))
        self.run_gate(True)

    def test_patch_from_shared_history_passes(self):
        git(self.repo, "revert", "--no-edit", "HEAD")
        git(self.repo, "branch", "-f", "release/0.3", "HEAD")
        # Reapply a patch that precedes the release branch fork.
        git(self.repo, "checkout", "-b", "candidate")
        self.commit("base", "base")
        self.run_gate(True)

    def test_empty_commit_fails(self):
        git(self.repo, "checkout", "-b", "candidate", "release/0.3")
        git(self.repo, "commit", "--allow-empty", "-m", "empty")
        self.run_gate(False)

    def test_novel_merge_resolution_fails(self):
        self.commit("fix", "upstream")
        git(self.repo, "checkout", "-b", "candidate", "release/0.3")
        git(self.repo, "merge", "--no-commit", "--no-ff", "main")
        (self.repo / "novel").write_text("merge-only change")
        git(self.repo, "add", "novel")
        git(self.repo, "commit", "-m", "merge")
        self.run_gate(False)

    def test_automation_has_no_upstream_exemption(self):
        git(self.repo, "checkout", "-b", "candidate", "release/0.3")
        self.commit("version", "1.2.3")
        pr = self.event["pull_request"]
        pr["head"]["ref"] = "release-please--branches--release/0.3"
        self.run_gate(False)
        pr["user"] = {"login": "tilr-bot[bot]", "type": "Bot"}
        self.run_gate(False)
        pr["head"]["repo"]["full_name"] = "fork/repo"
        self.run_gate(False)
        pr["head"]["repo"] = None
        self.run_gate(False)


if __name__ == "__main__":
    unittest.main()
