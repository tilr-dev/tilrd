"""Run in an isolated namespace: python3 -m unittest discover -s itests/tests."""

import errno
import os
import subprocess
import socket
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tt.port_stand import occupied_port
from scenarios.uc_033 import taken

TARGET = "uc_033:a_tilrd_tilr_starts_goes_on_without_the_taken_port"


class PortStandTest(unittest.TestCase):
    def test_target_holds_port_and_releases_after_failure(self):
        with self.assertRaises(OSError) as error:
            with occupied_port(TARGET, TARGET):
                with socket.socket() as probe:
                    probe.bind(("127.0.0.1", 7890))
                    self.fail("bind unexpectedly succeeded")
        self.assertEqual(error.exception.errno, errno.EADDRINUSE)
        with taken(7890):
            pass

    def test_runner_reproduces_uc033_and_preserves_failure(self):
        result = subprocess.run(
            [sys.executable, "-m", "tt", "case", TARGET],
            env={**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1]),
                 "TT_OCCUPY_PORT_FOR": TARGET},
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"[tt] port stand: {TARGET} holds 127.0.0.1:7890", result.stdout)
        self.assertIn("holder = taken(7890)", result.stdout)
        self.assertIn("Address already in use", result.stdout)
        with taken(7890):
            pass

    def test_disabled_and_other_case_have_no_holder(self):
        for case, target in [(TARGET, ""), ("uc_033:other", TARGET)]:
            with self.subTest(case=case, target=target):
                with occupied_port(case, target), taken(7890):
                    pass

    def test_success_closes_holder(self):
        with occupied_port(TARGET, TARGET):
            with socket.socket() as peer:
                peer.connect(("127.0.0.1", 7890))
        with taken(7890):
            pass

    def test_preexisting_holder_fails_setup_without_running_case(self):
        with taken(7890):
            with self.assertRaises(OSError) as error:
                with occupied_port(TARGET, TARGET):
                    self.fail("setup accepted an unrelated holder")
            self.assertEqual(error.exception.errno, errno.EADDRINUSE)
