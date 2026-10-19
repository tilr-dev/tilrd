#!/usr/bin/env python3
"""Integration check for the stand using a controlled, surviving fake server."""

from pathlib import Path
import os
import subprocess
import sys
import tempfile
import unittest


STAND = Path(__file__).with_name("test_server_cleanup_stand.py")


class CleanupStandTest(unittest.TestCase):
    def check_case(self, ready, fail_pgrep=False, clean_server=False):
        with tempfile.TemporaryDirectory(prefix="tilr-stand-selftest-") as scratch:
            root = Path(scratch)
            server = root / "target/debug/tilrd"
            server.parent.mkdir(parents=True)
            server.write_text(
                "#!/usr/bin/env python3\nimport sys\nsys.stdin.buffer.read(1)\n"
                if clean_server else "#!/usr/bin/env python3\nimport signal\nsignal.pause()\n"
            )
            server.chmod(0o755)
            pids = root / "pids"
            fixture = root / "fixture.py"
            fixture.write_text(
                "#!/usr/bin/env python3\n"
                "import subprocess, sys\n"
                f"child = subprocess.Popen([{str(server)!r}], "
                f"stdin=subprocess.{'PIPE' if clean_server else 'DEVNULL'}, "
                "stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
                f"with open({str(pids)!r}, 'a') as out: print(child.pid, file=out, flush=True)\n"
                + (
                    "print(f'TILR_STAND_READY pid={child.pid}', flush=True)\n"
                    "sys.stdin.buffer.read(1)\n"
                    if ready else "print('NO_READY', flush=True)\n"
                )
            )
            fixture.chmod(0o755)
            environment = os.environ.copy()
            if fail_pgrep:
                fake_bin = root / "bin"
                fake_bin.mkdir()
                fake_pgrep = fake_bin / "pgrep"
                fake_pgrep.write_text("#!/bin/sh\nexit 2\n")
                fake_pgrep.chmod(0o755)
                environment["PATH"] = str(fake_bin) + os.pathsep + environment["PATH"]
            result = subprocess.run(
                [sys.executable, str(STAND), "--executable", str(fixture)],
                text=True, capture_output=True, check=False, env=environment,
            )
            self.assertEqual(result.returncode, 0 if clean_server else 1,
                             result.stdout + result.stderr)
            self.assertEqual(len(pids.read_text().splitlines()), 2)
            if clean_server:
                self.assertIn("signal: server PID", result.stdout)
                self.assertIn("timeout: server PID", result.stdout)
                self.assertNotIn("running=True", result.stdout)
            elif fail_pgrep:
                self.assertEqual(result.stderr.count("pgrep failed"), 2)
            elif ready:
                self.assertIn("signal: test server", result.stderr)
                self.assertIn("timeout: test server", result.stderr)
            else:
                self.assertEqual(result.stderr.count("fixture exited before READY"), 2)
            matches = subprocess.run(
                ["pgrep", "-f", str(server)], text=True, capture_output=True, check=False,
            )
            self.assertEqual(matches.returncode, 1, matches.stdout + matches.stderr)

    def test_detects_survivors_and_cleans_them(self):
        self.check_case(ready=True)

    def test_cleans_before_ready(self):
        self.check_case(ready=False)

    def test_cleans_when_pgrep_fails(self):
        self.check_case(ready=True, fail_pgrep=True)

    def test_passes_when_server_exits_with_test(self):
        self.check_case(ready=True, clean_server=True)


if __name__ == "__main__":
    unittest.main()
