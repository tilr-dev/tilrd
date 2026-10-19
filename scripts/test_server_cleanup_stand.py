#!/usr/bin/env python3
"""Exercise test-server cleanup after a signal and a controlled timeout.

Run from anywhere: python3 scripts/test_server_cleanup_stand.py
The command fails if either server survives. All processes it starts are cleaned up.
"""

import argparse
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
READY = re.compile(rb"TILR_STAND_READY pid=(\d+)")


def build_fixture():
    command = [
        "cargo", "test", "-p", "tilrd", "--test", "cleanup_stand",
        "--no-run", "--locked", "--message-format=json-render-diagnostics",
    ]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError("fixture build failed:\n" + "\n".join(
            (result.stdout + "\n" + result.stderr).splitlines()[-40:]
        ))
    executables = [
        item["executable"]
        for line in result.stdout.splitlines()
        if (item := json.loads(line)).get("reason") == "compiler-artifact"
        and item.get("target", {}).get("name") == "cleanup_stand"
        and item.get("executable")
    ]
    if len(executables) != 1:
        raise RuntimeError(f"expected one cleanup_stand executable, got {executables!r}")
    return executables[0]


def ready_pid(process):
    """Read the fixture's confirmed-ready event; deadline only bounds a failure."""
    deadline = time.monotonic() + 30
    output = bytearray()
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError("fixture did not report READY within 30 seconds")
        readable, _, _ = select.select([process.stdout], [], [], remaining)
        if not readable:
            raise RuntimeError("fixture did not report READY within 30 seconds")
        chunk = os.read(process.stdout.fileno(), 4096)
        if not chunk:
            raise RuntimeError(f"fixture exited before READY: {output.decode(errors='replace')}")
        output.extend(chunk)
        match = READY.search(output)
        if match:
            pid = int(match.group(1))
            if pid <= 0 or os.getpgid(pid) != process.pid:
                raise RuntimeError(f"ready PID {pid} is not in the stand's process group")
            return pid


def server_running(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def mac_pgrep_contains(pid):
    result = subprocess.run(
        ["pgrep", "-f", "target/debug/tilrd"],
        text=True, capture_output=True, check=False,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(f"pgrep failed: {result.stderr.strip()}")
    return str(pid) in result.stdout.split()


def exit_watcher(pid):
    """Watch the exact PID without polling or sleeping where the OS supports it."""
    if sys.platform == "darwin":
        watcher = select.kqueue()
        try:
            watcher.control([select.kevent(
                pid, filter=select.KQ_FILTER_PROC,
                flags=select.KQ_EV_ADD | select.KQ_EV_ENABLE,
                fflags=select.KQ_NOTE_EXIT,
            )], 0, 0)
        except OSError:
            watcher.close()
            return None
        return watcher
    if sys.platform.startswith("linux") and hasattr(os, "pidfd_open"):
        try:
            return os.pidfd_open(pid)
        except OSError:
            return None
    return None


def wait_for_server_exit(watcher):
    if watcher is None:
        return False
    if isinstance(watcher, int):
        readable, _, _ = select.select([watcher], [], [], 2)
        return bool(readable)
    else:
        return bool(watcher.control(None, 1, 2))


def close_watcher(watcher):
    if isinstance(watcher, int):
        os.close(watcher)
    elif watcher is not None:
        watcher.close()


def run_case(executable, case):
    process = subprocess.Popen(
        [executable, "--ignored", "--exact", "server_ready_then_wait_for_interruption", "--nocapture"],
        cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, start_new_session=True,
    )
    server_pid = None
    watcher = None
    exited = False
    try:
        server_pid = ready_pid(process)
        watcher = exit_watcher(server_pid)
        if case == "signal":
            process.send_signal(signal.SIGTERM)
        else:
            try:
                process.wait(timeout=0.1)
            except subprocess.TimeoutExpired:
                process.kill()
            else:
                raise RuntimeError("fixture exited instead of reaching the controlled timeout")
        process.wait(timeout=5)
        exited = wait_for_server_exit(watcher)
        by_pid = not exited and server_running(server_pid)
        by_pgrep = mac_pgrep_contains(server_pid) if sys.platform == "darwin" else None
        print(f"{case}: server PID {server_pid}, running={by_pid}, mac_pgrep={by_pgrep}", flush=True)
        if by_pgrep is not None and by_pid != by_pgrep:
            raise RuntimeError(f"{case}: PID and pgrep disagree for server {server_pid}")
        if by_pid:
            raise RuntimeError(f"{case}: test server {server_pid} survived interruption")
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        finally:
            process.wait()
            try:
                if watcher is not None and not exited:
                    exited = wait_for_server_exit(watcher)
                if watcher is not None and not exited and server_running(server_pid):
                    raise RuntimeError(f"server {server_pid} did not exit after cleanup")
            finally:
                close_watcher(watcher)
                process.stdin.close()
                process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", help="already built cleanup_stand test executable")
    args = parser.parse_args()
    executable = args.executable or build_fixture()
    failures = []
    for case in ("signal", "timeout"):
        try:
            run_case(executable, case)
        except Exception as error:
            failures.append(str(error))
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
