"""Exercise the wrapper's failure path with a diagnostic-writing engine."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


WRAPPER = Path(__file__).resolve().parents[1] / "dt"


class DtTest(unittest.TestCase):
    def test_export_failure_preserves_stdout_and_stderr(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copyfile(WRAPPER, root / "dt")
            engine = root / "engine"
            engine.write_text('#!/bin/sh\necho "export diagnostic on stdout"\n'
                              'echo "export diagnostic on stderr" >&2\nexit 23\n')
            engine.chmod(0o755)
            result = subprocess.run(["sh", str(root / "dt"), "--jobs", "4"],
                                    env=dict(os.environ, DT_ENGINE=str(engine)),
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 23)
            self.assertIn("export diagnostic on stdout", result.stdout)
            self.assertIn("export diagnostic on stderr", result.stderr)
            self.assertNotIn("Report:", result.stdout)
            self.assertFalse((root / "results/latest").exists())


if __name__ == "__main__":
    unittest.main()
