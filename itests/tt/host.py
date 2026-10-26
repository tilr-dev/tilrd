"""A host: a container of its own (another IP, its own network stack) that a case drives.

Runs in a Dagger service container bound to the case under the host's name:

    python3 -m tt.host --port 7000

Every TCP connection on the port carries one request, a JSON line:

    {"op": "run", "argv": [...], "timeout": 60}  -> {"code", "out", "err"}, then closed
    {"op": "info"}                               -> {"ip": the host's address towards the
                                                     caller, "peer": the caller's address}
    {"op": "driver", "name": "wifi"}             -> {"ok": true}, then the connection is the
                                                     driver's stdin and stdout; closing it
                                                     kills the driver

The host runs as root with NET_ADMIN (insecure root capabilities), so a case can put
netem and iptables on its interfaces (tt.Net).
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import threading

DRIVER_EXE = os.environ.get("TT_DRIVER", "tilr-driver")
LOGS = "/tmp/tt"


def _serve(conn: socket.socket, addr):
    f = conn.makefile("rwb", buffering=0)
    try:
        req = json.loads(f.readline())
        op = req.get("op")
        if op == "run":
            try:
                p = subprocess.run(req["argv"], capture_output=True, text=True, timeout=req.get("timeout", 60),
                                   stdin=subprocess.DEVNULL)
                out = {"code": p.returncode, "out": p.stdout, "err": p.stderr}
            except FileNotFoundError as e:
                out = {"code": 127, "out": "", "err": f"{e}\n"}
            except subprocess.TimeoutExpired as e:
                out = {"code": 124, "out": str(e.stdout or ""), "err": f"timeout after {req.get('timeout')}s\n"}
            f.write((json.dumps(out) + "\n").encode())
        elif op == "info":
            f.write((json.dumps({"ip": conn.getsockname()[0], "peer": addr[0]}) + "\n").encode())
        elif op == "driver":
            _driver(conn, f, req.get("name", "driver"))
        else:
            f.write((json.dumps({"error": f"unknown op {op!r}"}) + "\n").encode())
    except (OSError, ValueError) as e:
        try:
            f.write((json.dumps({"error": str(e)}) + "\n").encode())
        except OSError:
            pass
    finally:
        try:
            conn.close()
        except OSError:
            pass


def _driver(conn: socket.socket, f, name: str):
    os.makedirs(LOGS, exist_ok=True)
    p = subprocess.Popen([DRIVER_EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=open(f"{LOGS}/{name}.stderr", "wb"), bufsize=0)
    f.write(b'{"ok": true}\n')

    def down():
        while chunk := os.read(p.stdout.fileno(), 65536):
            conn.sendall(chunk)
        try:
            conn.shutdown(socket.SHUT_WR)
        except OSError:
            pass

    threading.Thread(target=down, daemon=True).start()
    try:
        while chunk := conn.recv(65536):
            while chunk:
                chunk = chunk[os.write(p.stdin.fileno(), chunk):]
    except OSError:
        pass
    p.kill()
    p.wait()


def main():
    import argparse
    ap = argparse.ArgumentParser(description="tt host agent")
    ap.add_argument("--port", type=int, default=7000)
    args = ap.parse_args()
    srv = socket.create_server(("0.0.0.0", args.port), reuse_port=True)
    print(f"tt host agent on :{args.port}", flush=True)
    while True:
        conn, addr = srv.accept()
        threading.Thread(target=_serve, args=(conn, addr), daemon=True).start()


if __name__ == "__main__":
    main()
