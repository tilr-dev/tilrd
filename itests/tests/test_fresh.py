"""Exercise the embedded tt-fresh wrapper without importing the Dagger SDK."""

import ast
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


MODULE = Path(__file__).resolve().parents[1] / "src/tilr_itests/main.py"
FRESH = next(ast.literal_eval(node.value) for node in ast.parse(MODULE.read_text()).body
             if isinstance(node, ast.Assign)
             and any(isinstance(t, ast.Name) and t.id == "FRESH" for t in node.targets))


class FreshTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.targets = [self.root / "target", self.root / "target-next"]
        self.wrapper = self.root / "tt-fresh"
        self.wrapper.write_text(FRESH)

    def fresh(self, *command):
        return subprocess.run(
            [sys.executable, str(self.wrapper), *map(str, self.targets), "--", *command],
            cwd=self.source, capture_output=True, text=True,
        )

    def old_mtimes(self):
        for path in self.source.rglob("*"):
            if path.is_file():
                os.utime(path, (1, 1))

    def test_failed_build_does_not_certify_stale_next_generated_code(self):
        (self.source / "src").mkdir()
        (self.source / "proto").mkdir()
        (self.source / "Cargo.toml").write_text(
            '[package]\nname = "fresh-fixture"\nversion = "0.1.0"\nedition = "2021"\n')
        (self.source / "src/main.rs").write_text(
            'include!(concat!(env!("OUT_DIR"), "/schema.rs"));\n'
            'fn main() { println!("{}", VALUE); }\n')
        (self.source / "build.rs").write_text(
            'fn main() {\n'
            'println!("cargo:rerun-if-changed=proto/schema.rs");\n'
            'std::fs::copy("proto/schema.rs", '
            'std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("schema.rs")).unwrap();\n'
            '}\n')
        schema = self.source / "proto/schema.rs"
        schema.write_text("const VALUE: u32 = 1;\n")
        # Generate the lockfile before hashing, just as the stand's locked builds do.
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=self.source,
                       check=True, capture_output=True)
        self.old_mtimes()
        build_both = (
            "import subprocess, sys\n"
            "for target in sys.argv[1:]:\n"
            " subprocess.run(['cargo', 'run', '--offline', '--locked', '--quiet', "
            "'--target-dir', target], check=True)\n"
        )
        first = self.fresh(sys.executable, "-c", build_both, *map(str, self.targets))
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout.splitlines(), ["1", "1"])

        schema.write_text("const VALUE: u32 = 2;\n")
        self.old_mtimes()
        failed = self.fresh(sys.executable, "-c", build_both + "raise SystemExit(7)\n",
                            str(self.targets[0]))
        self.assertEqual(failed.returncode, 7, failed.stderr)
        # Dagger remounts the same bytes with their original (old) timestamps.
        self.old_mtimes()
        recovered = self.fresh(sys.executable, "-c", build_both, *map(str, self.targets))
        self.assertEqual(recovered.returncode, 0, recovered.stderr)
        self.assertEqual(recovered.stdout.splitlines(), ["2", "2"])

    def test_records_only_success_and_leaves_unchanged_sources_untouched(self):
        source = self.source / "input.proto"
        source.write_text("schema")
        result = self.fresh(sys.executable, "-c", "pass")
        self.assertEqual(result.returncode, 0, result.stderr)
        for target in self.targets:
            self.assertIn("input.proto", json.loads((target / ".tt-sources.json").read_text()))
        self.old_mtimes()
        result = self.fresh(sys.executable, "-c", "pass")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(source.stat().st_mtime_ns, 1_000_000_000)
        result = self.fresh(sys.executable, "-c", "raise SystemExit(9)")
        self.assertEqual(result.returncode, 9, result.stderr)
        for target in self.targets:
            self.assertFalse((target / ".tt-sources.json").exists())

    def test_interruption_invalidates_previous_success_before_command(self):
        (self.source / "input.proto").write_text("schema")
        self.assertEqual(self.fresh(sys.executable, "-c", "pass").returncode, 0)
        interrupted = self.fresh(
            sys.executable, "-c", "import os, signal; os.kill(os.getppid(), signal.SIGKILL)")
        self.assertEqual(interrupted.returncode, -9, interrupted.stderr)
        for target in self.targets:
            self.assertFalse((target / ".tt-sources.json").exists())


if __name__ == "__main__":
    unittest.main()
