#!/usr/bin/env python3
"""Exercise signing with a keg-only Homebrew installation and old LibreSSL."""
import base64
import json
import os
from pathlib import Path
import runpy
import shutil
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("sign-update-manifest.py")


class SignerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tilr-signer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.previous = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, self.previous)
        self.main = runpy.run_path(str(SCRIPT))["main"]
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.unsupported = self.bin / "openssl"
        self.unsupported.write_text(
            '#!/bin/sh\nif [ "$1" = version ]; then echo "LibreSSL 3.3.6"; '
            'else echo "unknown option -rawin" >&2; exit 1; fi\n'
        )
        self.unsupported.chmod(0o755)
        seed = bytes.fromhex("302e020100300506032b657004220420") + bytes(32)
        self.env = {
            "PATH": str(self.bin),
            "TILR_UPDATE_SIGNING_KEY": "-----BEGIN PRIVATE KEY-----\n"
            + base64.b64encode(seed).decode() + "\n-----END PRIVATE KEY-----\n",
            "TILR_UPDATE_PUBLIC_KEY": "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
        }
        archive = self.root / "dist/cdn/releases/9.8.7/tilr-9.8.7-linux-x86_64.tar.gz"
        archive.parent.mkdir(parents=True)
        archive.write_bytes(b"release archive")
        (self.root / "dist/cdn/releases/linux-x86_64").mkdir()

    def test_keg_only_openssl_signs_without_a_brew_bin_symlink(self):
        real_which = shutil.which
        real_openssl = os.environ.get("OPENSSL") or real_which(
            "openssl", path="/opt/homebrew/opt/openssl@3/bin:/usr/local/opt/openssl@3/bin:/opt/homebrew/bin:/usr/local/bin:" + os.environ["PATH"]
        )
        self.assertIsNotNone(real_openssl)
        for prefix in ["/opt/homebrew", "/usr/local"]:
            with self.subTest(prefix=prefix):
                keg = self.root / prefix.lstrip("/") / "opt/openssl@3/bin"
                keg.mkdir(parents=True)
                (keg / "openssl").symlink_to(real_openssl)

                def which(name, path=None):
                    # Keep only this architecture's keg; neither brew bin has a symlink.
                    paths = [str(keg) if p == f"{prefix}/opt/openssl@3/bin"
                             else str(self.root / "absent") for p in (path or "").split(":")]
                    return real_which(name, path=":".join(paths))

                with patch.dict(os.environ, self.env, clear=True), patch("sys.platform", "darwin"), patch("sys.argv", [str(SCRIPT), "9.8.7", "beta", "linux-x86_64"]), patch.object(shutil, "which", side_effect=which):
                    self.main()
                manifest = json.loads((self.root / "dist/cdn/releases/linux-x86_64/beta.manifest.json").read_text())
                self.assertEqual(len(bytes.fromhex(manifest["signature"])), 64)
                self.assertEqual(json.loads(manifest["payload"])["version"], "9.8.7")
                (keg / "openssl").unlink()

    def test_unsupported_override_reports_binary_and_version(self):
        self.env["OPENSSL"] = str(self.unsupported)
        with patch.dict(os.environ, self.env, clear=True), patch("sys.argv", [str(SCRIPT), "9.8.7", "latest", "linux-x86_64"]):
            with self.assertRaises(SystemExit) as error:
                self.main()
        self.assertIn(str(self.unsupported), str(error.exception))
        self.assertIn("LibreSSL 3.3.6", str(error.exception))
        self.assertIn("Ed25519", str(error.exception))
        self.assertFalse((self.root / "dist/cdn/releases/linux-x86_64/latest.manifest.json").exists())


if __name__ == "__main__":
    unittest.main()
