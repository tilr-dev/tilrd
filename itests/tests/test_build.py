"""Run the embedded build packager against a dependency-free Cargo workspace."""

import ast
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


MODULE = Path(__file__).resolve().parents[1] / "src/tilr_itests/main.py"
BUILD = next(ast.literal_eval(node.value) for node in ast.parse(MODULE.read_text()).body
             if isinstance(node, ast.Assign)
             and any(isinstance(t, ast.Name) and t.id == "BUILD" for t in node.targets))


class BuildTest(unittest.TestCase):
    def test_build_script_stdout_survives_without_the_build_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["tilrd", "tilr", "tilr-driver"]\nresolver = "2"\n')
            for name in ("tilrd", "tilr", "tilr-driver"):
                crate = source / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n'
                    '[features]\nfailpoints = []\n')
                (crate / "src/main.rs").write_text(
                    'fn main() { println!("fixture 0.1.0 (test)"); }\n')
                (crate / "build.rs").write_text(
                    'fn main() { println!("cargo:rerun-if-changed=build.rs"); }\n')
                (crate / "tests").mkdir()
                (crate / "tests/build_script.rs").write_text(
                    '#[test]\nfn reads_real_build_stdout() {\n'
                    'let path = std::path::Path::new(env!("OUT_DIR")).parent().unwrap().join("output");\n'
                    'let output = std::fs::read_to_string(path).unwrap();\n'
                    'assert!(output.contains("cargo:rerun-if-changed=build.rs"));\n}\n')

            # Relocate only the stand's absolute paths; execute the whole BUILD,
            # including its real cargo commands and JSON handling, unchanged.
            paths = {p: str(root / p.lstrip("/")) for p in ("/target-next", "/target", "/out")}

            class Relocate(ast.NodeTransformer):
                def visit_Constant(self, node):
                    if isinstance(node.value, str):
                        for original, relocated in paths.items():
                            if node.value == original or node.value.startswith(original + "/"):
                                return ast.copy_location(ast.Constant(
                                    relocated + node.value[len(original):]), node)
                    return node

            wrapper = root / "tt-build"
            wrapper.write_text(ast.unparse(Relocate().visit(ast.parse(BUILD))))
            env = dict(os.environ, CARGO_TARGET_DIR=paths["/target"],
                       TT_NEXT_PACKAGE_VERSION="0.1.1", CARGO_NET_OFFLINE="true")
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=source,
                           env=env, capture_output=True, check=True)
            stale = root / "target/debug/build/stale-hash/output"
            stale.parent.mkdir(parents=True)
            stale.write_text("must not be exported")
            for phase in ("initial", "cached"):
                with self.subTest(phase=phase):
                    shutil.rmtree(root / "out", ignore_errors=True)
                    built = subprocess.run([sys.executable, str(wrapper)], cwd=source,
                                           env=env, capture_output=True, text=True)
                    self.assertEqual(built.returncode, 0, built.stderr)
                    snapshot = root / "out/rust-target"
                    self.assertFalse((snapshot / stale.relative_to(root / "target")).exists())
                    # Hide all original Cargo artifacts and restore only the export.
                    cache = root / "hidden-cache"
                    (root / "target").rename(cache)
                    try:
                        shutil.copytree(snapshot, root / "target")
                        manifest = json.loads((root / "out/manifest.json").read_text())
                        suites = [s for s in manifest["suites"] if s["suite"].endswith("/build_script")]
                        self.assertEqual(len(suites), 3)
                        for suite in suites:
                            run = subprocess.run([suite["exe"]], cwd=source,
                                                 capture_output=True, text=True)
                            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
                    finally:
                        shutil.rmtree(root / "target", ignore_errors=True)
                        cache.rename(root / "target")


if __name__ == "__main__":
    unittest.main()
