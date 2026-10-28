#!/usr/bin/env python3
"""Run the release version gate against isolated Cargo workspaces."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-release-version.py").resolve()


class ReleaseVersion(unittest.TestCase):
    def test_tag_selection_and_cargo_files_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = '[workspace.package]\nversion = "0.3.2"\n'
            lock = '# lock fixture\n'
            (root / 'Cargo.toml').write_text(manifest)
            (root / 'Cargo.lock').write_text(lock)
            for tag, success in [('v0.3.2', True), ('v0.3.3-rc.1', True),
                                 ('v0.10.0', True), ('v1.0.0', True),
                                 ('v0.3.1', False), ('v0.2.99-rc.9', False),
                                 ('v00.3.2', False), ('v0.3.3-rc.0', False),
                                 ('v0.3.3-beta.1', False), ('0.3.3', False)]:
                with self.subTest(tag=tag):
                    result = subprocess.run([sys.executable, str(SCRIPT), tag], cwd=root,
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode == 0, success, result.stderr)
                    self.assertEqual((root / 'Cargo.toml').read_text(), manifest)
                    self.assertEqual((root / 'Cargo.lock').read_text(), lock)


if __name__ == '__main__':
    unittest.main()
