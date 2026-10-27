#!/usr/bin/env python3
"""Integration check for the stand using a controlled, surviving fake server."""

import importlib.util
from pathlib import Path
import os
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch


STAND = Path(__file__).with_name("test_server_cleanup_stand.py")
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("cleanup_stand", STAND)
stand = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stand)


class ReadyEventTest(unittest.TestCase):
    def test_disappeared_pid_has_explicit_error(self):
        process = Mock(pid=123)
        with patch.object(stand.select, "select", return_value=([process.stdout], [], [])), \
                patch.object(stand.os, "read", return_value=b"TILR_STAND_READY pid=12345\n"), \
                patch.object(stand.os, "getpgid", side_effect=ProcessLookupError):
            with self.assertRaisesRegex(RuntimeError, "ready PID 12345 is not alive"):
                stand.ready_pid(process)

    def test_pid_split_across_pipe_reads(self):
        process = Mock(pid=123)
        with patch.object(stand.select, "select", return_value=([process.stdout], [], [])), \
                patch.object(stand.os, "read", side_effect=[
                    b"fixture output\nTILR_STAND_READY pid=12", b"345\n",
                ]) as read, patch.object(stand.os, "getpgid", return_value=process.pid):
            self.assertEqual(stand.ready_pid(process), 12345)
            self.assertEqual(read.call_count, 2)


class ExitEventTest(unittest.TestCase):
    def test_watcher_receives_full_exit_deadline(self):
        # The controlled watcher exposes its event only with the full deadline.
        class ControlledWatcher:
            def control(self, changes, count, timeout):
                return [object()] if timeout == stand.EXIT_DEADLINE else []

        self.assertTrue(stand.wait_for_server_exit(ControlledWatcher()))
        with patch.object(stand.select, "select", side_effect=lambda r, w, x, timeout:
                          (r if timeout == stand.EXIT_DEADLINE else [], [], [])):
            self.assertTrue(stand.wait_for_server_exit(123))

    def test_missing_watcher_is_an_error(self):
        with self.assertRaisesRegex(RuntimeError, "exit watcher"):
            stand.wait_for_server_exit(None)

    def test_unsupported_watcher_is_an_error(self):
        with patch.object(stand.sys, "platform", "unsupported"):
            with self.assertRaisesRegex(RuntimeError, "exit watcher"):
                stand.exit_watcher(123)


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
            directories = root / "directories"
            fixture = root / "fixture.py"
            fixture.write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, socket, subprocess, sys\n"
                f"scratch = pathlib.Path(os.environ.get('TILR_STAND_DIR', {str(root)!r} + '/unowned-' + str(os.getpid())))\n"
                "scratch.mkdir(exist_ok=True)\n"
                "notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)\n"
                "notify.bind(str(scratch / 'notify.sock'))\n"
                f"with open({str(directories)!r}, 'a') as out: print(scratch, file=out, flush=True)\n"
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
            for directory in directories.read_text().splitlines():
                self.assertFalse(Path(directory).exists(), f"fixture artifacts remain: {directory}")
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

    # Model an orphan zombie: kill(pid, 0) can still succeed after exit.
    @patch.object(stand, "server_running", return_value=True)
    def test_cleans_when_stand_receives_sigterm(self, _pid_probe):
        with tempfile.TemporaryDirectory(prefix="tilr-stand-signal-") as scratch:
            root = Path(scratch)
            pids = root / "pids"
            fixture = root / "fixture.py"
            fixture.write_text(
                "#!/usr/bin/env python3\nimport os, subprocess, sys\n"
                "child = subprocess.Popen([sys.executable, '-c', "
                "'import signal; signal.pause()'])\n"
                f"with open({str(pids)!r}, 'w') as out: "
                "print(os.getpid(), child.pid, os.environ.get('TILR_STAND_DIR', ''), file=out, flush=True)\n"
                "print(f'TILR_STAND_READY pid={child.pid}', flush=True)\n"
                "sys.stdin.buffer.read(1)\n"
            )
            fixture.chmod(0o755)
            event_read, event_write = os.pipe()
            # Signal only after the ready fixture has exited and the stand has
            # registered the server watcher. The pipe is the ordering event.
            wrapper = (
                "import sys; sys.dont_write_bytecode = True; "
                f"sys.path.insert(0, {str(STAND.parent)!r}); "
                "import test_server_cleanup_stand as stand; import os\n"
                "original = stand.wait_for_server_exit\n"
                "def notify(watcher):\n"
                f"    os.write({event_write}, b'R')\n"
                "    return original(watcher)\n"
                "stand.wait_for_server_exit = notify\n"
                "sys.exit(stand.main())\n"
            )
            process = subprocess.Popen(
                [sys.executable, "-c", wrapper, "--executable", str(fixture)],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                pass_fds=(event_write,),
            )
            os.close(event_write)
            watcher = None
            try:
                self.assertTrue(select.select([event_read], [], [], 30)[0],
                                "stand did not reach its server exit check")
                self.assertEqual(os.read(event_read, 1), b"R")
                _, pid, directory = pids.read_text().split()
                server_pid = int(pid)
                watcher = stand.exit_watcher(server_pid)
                process.send_signal(signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=40)
                self.assertTrue(stand.wait_for_server_exit(watcher),
                                f"server {server_pid} survived stand SIGTERM")
            finally:
                stand.close_watcher(watcher)
                os.close(event_read)
                if process.poll() is None:
                    process.kill()
                process.communicate()
                # Also clean the deliberately failing regression run.
                if pids.exists():
                    fixture_pid = int(pids.read_text().split()[0])
                    try:
                        os.killpg(fixture_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            self.assertEqual(process.returncode, 143, stdout + stderr)
            self.assertFalse(Path(directory).exists(), f"fixture artifacts remain: {directory}")

    def test_detects_survivors_and_cleans_them(self):
        self.check_case(ready=True)

    def test_cleans_before_ready(self):
        self.check_case(ready=False)

    @unittest.skipUnless(sys.platform == "darwin", "stand uses pgrep only on macOS")
    def test_cleans_when_pgrep_fails(self):
        self.check_case(ready=True, fail_pgrep=True)

    def test_passes_when_server_exits_with_test(self):
        self.check_case(ready=True, clean_server=True)


if __name__ == "__main__":
    unittest.main()
