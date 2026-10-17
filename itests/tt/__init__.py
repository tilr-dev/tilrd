"""tt: fixtures and scenario cases for tilrd, black box.

A case drives the real server binary through tilr-driver (a scripted client), looks at
processes the way a user would (ps, /proc), and can put a fault-injecting proxy or a
service (a fake CDN) between the parts. The helper API names what a case needs, not how
a backend provides it, so the same cases can later run on the `local` backend (plain
processes on a Mac); what a backend cannot give is asked for with `t.need(...)` and the
case is BLOCKED there.

Layout of a container (backend `dagger`, see src/tilr_itests/main.py):

    /fx                    the fixture snapshot; the case's working directory
    /tmp/tt                scratch: servers' runtime/state/config dirs, logs
    tilr-server            the server under test, on PATH (no failpoints)
    tilr-driver            the scripted client, on PATH
    /opt/tt/tilr-server-next   the same source built as the next version
    /opt/tt/manifest.json  versions and platform of both builds
    $TT_SERVICE_<NAME>     URL of a service container the case asked for (cdn)
"""

from __future__ import annotations

import dataclasses
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import threading
import time
from pathlib import Path
from typing import Callable

FX = Path(os.environ.get("TT_FX", "/fx"))
TMP = Path(os.environ.get("TT_TMP", "/tmp/tt"))
BACKEND = os.environ.get("TT_BACKEND", "dagger")
METRICS = Path(os.environ.get("TT_METRICS", "/tt-metrics.json"))
SERVER_EXE = os.environ.get("TT_SERVER", "tilr-server")
DRIVER_EXE = os.environ.get("TT_DRIVER", "tilr-driver")
NEXT_EXE = os.environ.get("TT_SERVER_NEXT", "/opt/tt/tilr-server-next")
MANIFEST = Path(os.environ.get("TT_MANIFEST", "/opt/tt/manifest.json"))

# What each backend can give a case; t.need() turns a missing one into BLOCKED.
CAPABILITIES = {
    "dagger": {"linux", "procfs", "container", "services", "root", "hosts", "netem"},
    "local": set(),
}

FIXTURES: dict[str, "Fixture"] = {}
CASES: list["Case"] = []


@dataclasses.dataclass
class Fixture:
    name: str
    module: str
    requires: list[str]
    tilr: bool
    build: Callable
    check: Callable | None = None
    doc: str = ""


@dataclasses.dataclass
class Case:
    id: str
    module: str
    fixture: str | None
    services: list[str]
    timeout: int
    fn: Callable
    title: str
    hosts: list[str] = dataclasses.field(default_factory=list)
    net_admin: bool = False


def fixture(*, requires: tuple[str, ...] | list[str] = (), tilr: bool = False):
    """Register a fixture. tilr=False: the binaries are absent, so the snapshot survives
    any code change. Decorate a check with @<fixture>.check."""
    def deco(fn):
        fx = Fixture(fn.__name__, fn.__module__, list(requires), tilr, fn, doc=(fn.__doc__ or "").strip())
        if fx.name in FIXTURES:
            raise ValueError(f"duplicate fixture {fx.name}")
        FIXTURES[fx.name] = fx

        def check(chk):
            fx.check = chk
            return chk
        fn.check = check
        return fn
    return deco


def case(*, fixture: str | None = None, services: tuple[str, ...] | list[str] = (), timeout: int = 300,
         hosts: tuple[str, ...] | list[str] = (), net_admin: bool = False):
    """Register a scenario case: a fresh container (copy of the fixture snapshot), the
    services it names bound by hostname, killed after `timeout` seconds. `hosts`: containers
    of their own the case drives (t.host(name)); `net_admin`: the case's own container may
    change its network (t.net())."""
    def deco(fn):
        short = fn.__module__.rsplit(".", 1)[-1]
        title = (fn.__doc__ or "").strip().splitlines()[0] if fn.__doc__ else ""
        CASES.append(Case(f"{short}:{fn.__name__}", fn.__module__, fixture, list(services), timeout, fn, title,
                          list(hosts), net_admin))
        return fn
    return deco


class Blocked(Exception):
    """The case needs something this backend or runtime does not have."""


class Failure(AssertionError):
    pass


@dataclasses.dataclass
class R:
    argv: list[str]
    code: int
    out: str
    err: str

    def json(self):
        return json.loads(self.out)

    @property
    def text(self) -> str:
        return self.out + self.err


def _indent(text: str, limit: int = 60) -> str:
    lines = text.rstrip("\n").splitlines()
    if len(lines) > limit:
        lines = lines[: limit // 2] + [f"... {len(lines) - limit} lines ..."] + lines[-limit // 2:]
    return "".join(f"    {l}\n" for l in lines)


def _short(value, limit: int = 300) -> str:
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
    return text if len(text) <= limit else text[:limit] + f"… ({len(text)} chars)"


_T0 = time.monotonic()


def stamp() -> str:
    return f"[{time.monotonic() - _T0:8.3f}]"


# ---------------------------------------------------------------------------- processes


@dataclasses.dataclass
class Proc:
    """A process as /proc shows it."""
    pid: int
    ppid: int
    pgid: int
    sid: int
    tty: int
    state: str
    comm: str
    cmdline: str
    rss_kb: int
    fds: int

    @property
    def alive(self) -> bool:
        return self.state not in ("Z", "X")


def proc(pid: int) -> Proc | None:
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
        cmdline = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").strip()
        status = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return None
    comm = stat[stat.index("(") + 1: stat.rindex(")")]
    fields = stat[stat.rindex(")") + 2:].split()
    rss = next((int(l.split()[1]) for l in status.splitlines() if l.startswith("VmRSS:")), 0)
    try:
        fds = len(os.listdir(f"/proc/{pid}/fd"))
    except OSError:
        fds = -1
    return Proc(pid, int(fields[1]), int(fields[2]), int(fields[3]), int(fields[4]), fields[0], comm, cmdline,
                rss, fds)


def procs() -> list[Proc]:
    out = []
    for name in os.listdir("/proc"):
        if name.isdigit() and (p := proc(int(name))):
            out.append(p)
    return out


def descendants(pid: int) -> list[Proc]:
    every = procs()
    found, frontier = [], {pid}
    while frontier:
        kids = [p for p in every if p.ppid in frontier]
        found += kids
        frontier = {p.pid for p in kids}
    return found


# ---------------------------------------------------------------------------- server


class Server:
    """A tilrd the case started (or found): its directories, log, address and token."""

    def __init__(self, t: "T", name: str, exe: str, dir: Path, env: dict, log: Path,
                 popen: subprocess.Popen | None = None):
        self.t, self.name, self.exe, self.dir, self.env, self.log = t, name, exe, dir, env, log
        self.popen = popen
        self.url = ""
        self.pid = popen.pid if popen else 0

    def wait_ready(self, timeout: float = 30) -> "Server":
        """Wait for `listening on ws://...` in the log."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            m = re.search(r"listening on (ws://\S+)", self.log_text())
            if m:
                self.url = m.group(1)
                break
            if self.popen and self.popen.poll() is not None:
                raise Failure(f"{self.name} exited {self.popen.returncode} before listening:\n{self.log_text()}")
            time.sleep(0.05)
        else:
            raise Failure(f"{self.name} not listening after {timeout}s:\n{self.log_text()}")
        if not self.pid:
            self.pid = self.t.listener_pid(self.port)
        print(f"{stamp()} {self.name} ready at {self.url} (pid {self.pid})")
        return self

    @property
    def port(self) -> int:
        return int(self.url.rsplit(":", 1)[1])

    def token(self) -> str:
        return self.t.run(self.exe, "token", env=self.env, quiet=True, check=True).out.strip()

    def cmd(self, *args, **kw) -> R:
        """Run a tilr-server subcommand with this server's environment."""
        return self.t.run(self.exe, *args, env={**self.env, **kw.pop("env", {})}, **kw)

    def version(self) -> str:
        return self.cmd("--version", quiet=True, check=True).out.split()[1]

    def log_text(self) -> str:
        try:
            return self.log.read_text(errors="replace")
        except OSError:
            return ""

    def logged(self, needle: str) -> int:
        return self.log_text().count(needle)

    def alive(self) -> bool:
        p = proc(self.pid)
        return bool(p and p.alive)

    def panes(self) -> list[Proc]:
        """Pane processes (`tilr-server pane --id ...`) under the supervisor."""
        return [p for p in descendants(self.pid) if " pane --id " in f" {p.cmdline} "]

    def signal(self, sig: int):
        os.kill(self.pid, sig)

    def stop(self):
        if not self.pid or not self.alive():
            return
        try:
            os.kill(self.pid, signal.SIGTERM)
        except OSError:
            return
        for _ in range(100):
            if not self.alive():
                return
            time.sleep(0.05)
        try:
            os.kill(self.pid, signal.SIGKILL)
        except OSError:
            pass


# ---------------------------------------------------------------------------- driver


class Driver:
    """A tilr-driver process: one client, commands in, events out."""

    def __init__(self, t: "T", name: str, host: "Host | None" = None):
        self.t, self.name, self.host = t, name, host
        self.stderr = TMP / (f"{host.name}-{name}.stderr" if host else f"{name}.stderr")
        if host is None:
            self.p = subprocess.Popen([DRIVER_EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                      stderr=open(self.stderr, "wb"), text=True, bufsize=1)
            self._w, self._r = self.p.stdin, self.p.stdout
        else:
            self.p = None
            self.sock = host.attach_driver(name)
            self._w = self.sock.makefile("w", buffering=1, encoding="utf-8")
            self._r = self.sock.makefile("r", encoding="utf-8")
        self.events: list[dict] = []
        self.cond = threading.Condition()
        self.next_id = 0
        self.server: Server | None = None
        threading.Thread(target=self._read, daemon=True).start()
        self.wait_event("ready", timeout=10)

    def _read(self):
        for line in self._r:
            try:
                ev = json.loads(line)
            except ValueError:
                ev = {"ev": "garbage", "line": line}
            ev["_at"] = time.monotonic()  # when the case saw it, on the case's clock
            if "id" not in ev:
                print(f"{stamp()} {self.name} <- {_short({k: v for k, v in ev.items() if k != '_at'})}")
            with self.cond:
                self.events.append(ev)
                self.cond.notify_all()
        with self.cond:
            self.events.append({"ev": "exited"})
            self.cond.notify_all()

    def wait_event(self, ev: str | tuple[str, ...], timeout: float = 10, since: int = 0,
                   pred: Callable[[dict], bool] = lambda e: True) -> dict:
        """The first event named `ev` (or any of several) at or after index `since` that
        `pred` accepts."""
        names = ev if isinstance(ev, tuple) else (ev,)
        deadline = time.monotonic() + timeout
        with self.cond:
            while True:
                for e in self.events[since:]:
                    if e.get("ev") in names and pred(e):
                        return e
                left = deadline - time.monotonic()
                if left <= 0:
                    raise Failure(f"{self.name}: no {ev!r} event within {timeout}s")
                self.cond.wait(left)

    def mark(self) -> int:
        """Index for wait_event(since=...): only events after now."""
        with self.cond:
            return len(self.events)

    def cmd(self, name: str, timeout: float | None = None, ok: bool = True, quiet: bool = False, **args) -> dict:
        """Send a command and return its answer; an `error` answer fails the case unless ok=False."""
        self.next_id += 1
        cid = self.next_id
        line = json.dumps({"id": cid, "cmd": name, **args})
        if not quiet:
            print(f"{stamp()} {self.name} -> {_short(line, 200)}")
        self._w.write(line + "\n")
        self._w.flush()
        reply = self._answer(cid, timeout or (args.get("timeout_ms", 0) / 1000 + 30))
        if not quiet:
            print(f"{stamp()} {self.name} <- {_short({k: v for k, v in reply.items() if k != '_at'})}")
        if ok and reply.get("ev") == "error":
            raise Failure(f"{self.name} {name}: {reply.get('message')}")
        return reply

    def _answer(self, cid: int, timeout: float) -> dict:
        deadline = time.monotonic() + timeout
        with self.cond:
            while True:
                for e in reversed(self.events):
                    if e.get("id") == cid:
                        return e
                if self.events and self.events[-1].get("ev") == "exited":
                    raise Failure(f"{self.name} exited:\n{self.stderr_text()}")
                left = deadline - time.monotonic()
                if left <= 0:
                    raise Failure(f"{self.name}: no answer to command {cid} within {timeout}s")
                self.cond.wait(left)

    # ---- conveniences

    def connect(self, target: "Server | str", token: str | None = None, **kw) -> dict:
        if isinstance(target, Server):
            self.server = target
            url, token = target.url, token if token is not None else target.token()
        else:
            url = target
        return self.cmd("connect", url=url, **({"token": token} if token is not None else {}), **kw)

    def create_pane(self, cols: int = 100, rows: int = 30, **kw) -> str:
        return self.cmd("create_pane", cols=cols, rows=rows, **kw)["pane"]

    def shell_pane(self, cols: int = 100, rows: int = 30) -> str:
        """A pane with the login shell, once its prompt is up."""
        pane = self.create_pane(cols, rows)
        self.wait_for(pane, r"(?m)^\$ ?$", timeout=15)
        return pane

    def input(self, pane: str, text: str) -> dict:
        return self.cmd("input", pane=pane, text=text)

    def line(self, pane: str, text: str) -> dict:
        return self.input(pane, text + "\r")

    def key(self, pane: str, key: str) -> dict:
        return self.cmd("key", pane=pane, key=key)

    def interrupt(self, pane: str, timeout: float = 30) -> None:
        """Ctrl-C the pane's foreground program until the shell's prompt is back. Text typed
        before the program died is flushed with it, so what comes next waits for this."""
        deadline = time.monotonic() + timeout
        while True:
            self.key(pane, "ctrl-c")
            left = max(0.5, min(3, deadline - time.monotonic()))
            if self.wait_for(pane, r"(?m)^\$ ?$", timeout=left, ok=False).get("ev") == "matched":
                return
            if time.monotonic() >= deadline:
                raise Failure(f"{self.name}: no prompt after Ctrl-C in {pane}")

    def screen(self, pane: str) -> dict:
        return self.cmd("screen", pane=pane, quiet=True)

    def text(self, pane: str) -> str:
        return self.screen(pane)["text"]

    def wait_for(self, pane: str, regex: str, timeout: float = 10, ok: bool = True) -> dict:
        return self.cmd("wait_for", pane=pane, regex=regex, timeout_ms=int(timeout * 1000), ok=ok)

    def list(self) -> list[dict]:
        return self.cmd("list")["panes"]

    def viewers(self, pane: str) -> int:
        """How many clients the server says watch the pane."""
        return next((p["viewers"] for p in self.cmd("list", quiet=True)["panes"] if p["pane"] == pane), -1)

    def latency(self, reset: bool = False) -> dict:
        """Input latency so far, as the driver measured it (send -> first frame carrying the
        input's seq): count, pending, unanswered, p50_ms, p90_ms, p99_ms, max_ms."""
        return self.cmd("latency", reset=reset, quiet=True)

    def connected(self) -> bool:
        return self.cmd("status", quiet=True)["connected"]

    _n = 0

    def run(self, pane: str, command: str, timeout: float = 20) -> int:
        """Run a command in the pane's shell; its exit status. The marker is typed split
        in two, so only the shell's output matches."""
        Driver._n += 1
        n = Driver._n
        self.line(pane, f'{command}; echo "<st""atus{n}:$?>"')
        return int(self.wait_for(pane, rf"<status{n}:(\d+)>", timeout)["groups"][1])

    def value(self, pane: str, expr: str, timeout: float = 20) -> str:
        """What the pane's shell prints for `expr` (shell syntax, one line)."""
        Driver._n += 1
        n = Driver._n
        self.line(pane, f'echo "<v""{n}>{expr}</v{n}>"')
        return self.wait_for(pane, rf"<v{n}>(.*?)</v{n}>", timeout)["groups"][1]

    def stderr_text(self) -> str:
        if self.host:
            text = self.host.run("cat", f"/tmp/tt/{self.name}.stderr", quiet=True).out
            self.stderr.write_text(text)
            return text
        return self.stderr.read_text(errors="replace")

    def quit(self):
        if self.p is None:
            import socket
            try:
                self._w.write(json.dumps({"cmd": "quit"}) + "\n")
                self._w.flush()
                self.wait_event("bye", timeout=5)
                self.sock.shutdown(socket.SHUT_WR)
                self.wait_event("exited", timeout=2)
            except (OSError, ValueError, Failure):
                pass
            self.sock.close()
            try:
                self.stderr_text()
            except Exception:
                pass
        elif self.p.poll() is None:
            try:
                self.p.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
                self.p.stdin.flush()
                self.p.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired, ValueError):
                self.p.kill()


# ---------------------------------------------------------------------------- a counter in a pane


class Counter:
    """A counter running in a pane's shell: `tick N` on the screen every `period` seconds,
    and `N <unix time>` appended to `log`, a file on the server's machine (this container)
    the case reads directly: what the pane did, whatever reached the client."""

    def __init__(self, t: "T", driver: Driver, pane: str, period: float = 0.1, name: str = "ticks"):
        self.t, self.pane = t, pane
        self.log = t.scratch("counter") / name
        driver.line(pane, f"i=0; while :; do i=$((i+1)); echo \"tick $i\"; echo \"$i $(date +%s.%N)\" >>{self.log}; "
                          f"sleep {period}; done")

    @staticmethod
    def on_screen(text: str) -> list[int]:
        return [int(m) for m in re.findall(r"(?m)^tick (\d+)$", text)]

    def last(self, driver: Driver) -> int:
        """The highest tick on the driver's screen; 0 while it has no screen (reconnecting)."""
        screen = driver.cmd("screen", pane=self.pane, ok=False, quiet=True, timeout_ms=1000)
        seen = self.on_screen(screen.get("text", ""))
        return max(seen) if seen else 0

    def check_screen(self, driver: Driver, ahead: int = 5, timeout: float = 20, what: str = "the screen"):
        """The counter moves on the driver's screen, whose tick lines are consecutive:
        nothing lost, nothing doubled."""
        start = self.t.eventually(lambda: self.last(driver), f"a counter on {what}", timeout=timeout)
        self.t.eventually(lambda: self.last(driver) >= start + ahead, f"the counter moving on {what}",
                          timeout=timeout, interval=0.05)
        seen = self.on_screen(driver.text(self.pane))
        self.t.eq(seen, list(range(seen[0], seen[0] + len(seen))), f"counter lines on {what} are consecutive")

    def written(self) -> list[tuple[int, float]]:
        return [(int(n), float(s)) for n, s in (line.split() for line in self.log.read_text().splitlines() if line)]

    def check_log(self, max_gap: float = 1.0) -> float:
        """The pane's counter skipped nothing and never stalled longer than max_gap (it
        does not wait for a client); the longest gap, recorded as counter_max_gap_ms."""
        written = self.written()
        numbers = [n for n, _ in written]
        self.t.eq(numbers, list(range(1, len(numbers) + 1)), "the counter skipped nothing")
        gap = max((b - a for (_, a), (_, b) in zip(written, written[1:])), default=0.0)
        self.t.metric("counter_max_gap_ms", round(gap * 1000))
        self.t.ok(gap < max_gap, f"the counter never stalled (longest gap {gap:.2f}s, limit {max_gap}s)")
        return gap


# ---------------------------------------------------------------------------- terminal session


class Session:
    """A terminal login session, as SSH gives one: a pty, a shell as session leader with
    the pty as its controlling terminal. hangup() is the SSH connection dropping."""

    def __init__(self, t: "T", argv: list[str], env: dict, name: str):
        import pty
        self.t, self.name = t, name
        self.master, slave = pty.openpty()
        os.set_blocking(self.master, False)
        # setsid -c: a new session whose controlling terminal is stdin (the pty).
        self.p = subprocess.Popen(["setsid", "-c", *argv], stdin=slave, stdout=slave, stderr=slave,
                                  env=env, close_fds=True)
        os.close(slave)
        self.pid = self.p.pid
        self.output = b""

    def read(self) -> str:
        try:
            while chunk := os.read(self.master, 65536):
                self.output += chunk
        except (BlockingIOError, OSError):
            pass
        return self.output.decode(errors="replace")

    def write(self, text: str):
        os.write(self.master, text.encode())

    def expect(self, regex: str, timeout: float = 10) -> re.Match:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            m = re.search(regex, self.read())
            if m:
                return m
            time.sleep(0.05)
        raise Failure(f"{self.name}: no /{regex}/ within {timeout}s; terminal:\n{self.read()[-3000:]}")

    def hangup(self):
        """Close the terminal (the kernel sends SIGHUP to the session's controlling process
        and its foreground group), then SIGHUP the shell's own process group, as sshd does."""
        p = proc(self.pid)
        pgid = p.pgid if p else self.pid
        os.close(self.master)
        try:
            os.killpg(pgid, signal.SIGHUP)
        except OSError:
            pass
        print(f"{stamp()} {self.name}: hung up (pty closed, SIGHUP to group {pgid})")


# ---------------------------------------------------------------------------- hosts and the network


class Host:
    """A container of its own the case drives (tt/host.py): another IP, its own network
    stack, root with NET_ADMIN. Bound under its name; the agent listens on port 7000."""

    def __init__(self, t: "T", name: str, addr: str):
        self.t, self.name = t, name
        host, _, port = addr.rpartition(":")
        self.addr = (host, int(port))
        info = self._request({"op": "info"}, timeout=10)
        self.ip, self.peer_ip = info["ip"], info["peer"]
        self.net = Net(t, self.run, name)

    def _connect(self, timeout: float):
        import socket
        deadline = time.monotonic() + 30
        while True:
            try:
                return socket.create_connection(self.addr, timeout=timeout)
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.2)

    def _request(self, req: dict, timeout: float) -> dict:
        with self._connect(timeout) as sock:
            sock.sendall((json.dumps(req) + "\n").encode())
            answer = sock.makefile("rb").readline()
        return json.loads(answer)

    def run(self, *argv, timeout: float = 60, quiet: bool = False, check: bool = False) -> R:
        argv = [str(a) for a in argv]
        out = self._request({"op": "run", "argv": argv, "timeout": timeout}, timeout + 10)
        r = R(argv, out.get("code", 1), out.get("out", ""), out.get("err", out.get("error", "")))
        if not quiet:
            print(f"{stamp()} {self.name}$ {shlex.join(argv)}")
            if r.out:
                print(_indent(r.out), end="")
            if r.err:
                print("  stderr:\n" + _indent(r.err), end="")
            print(f"  -> exit {r.code}")
        if check and r.code != 0:
            raise Failure(f"{self.name}: {shlex.join(argv)} exited {r.code}\n{r.text}")
        return r

    def sh(self, script: str, **kw) -> R:
        return self.run("bash", "-c", script, **kw)

    def attach_driver(self, name: str):
        sock = self._connect(None)
        sock.settimeout(None)
        sock.sendall((json.dumps({"op": "driver", "name": name}) + "\n").encode())
        # The agent's answer is one line; read it byte by byte so nothing of the
        # driver's own output is swallowed by a buffer.
        line = b""
        while not line.endswith(b"\n"):
            b = sock.recv(1)
            if not b:
                raise Failure(f"{self.name}: the host closed the driver connection")
            line += b
        if not json.loads(line).get("ok"):
            raise Failure(f"{self.name}: no driver: {line!r}")
        return sock

    def driver(self, name: str = "driver") -> "Driver":
        d = Driver(self.t, name, host=self)
        self.t._cleanup.append(d.quit)
        return d


class Net:
    """Layer 2 faults on a container's own network stack: netem on its egress, iptables
    DROP / REJECT of a peer, killing sockets. `run` runs a command there as root."""

    CHAIN = "TT"

    def __init__(self, t: "T", run: Callable, where: str):
        self.t, self._run, self.where = t, run, where
        self._dev = ""
        self._chain = False

    def sh(self, script: str, check: bool = True) -> R:
        return self._run("sh", "-c", script, check=check)

    @property
    def dev(self) -> str:
        """The interface of the default route (eth0 in a container)."""
        if not self._dev:
            out = self._run("ip", "-4", "route", "show", "default", quiet=True).out
            m = re.search(r"\bdev (\S+)", out)
            self._dev = m.group(1) if m else "eth0"
        return self._dev

    def ip(self) -> str:
        out = self._run("ip", "-4", "-o", "addr", "show", "dev", self.dev, quiet=True).out
        return re.search(r"inet (\d+\.\d+\.\d+\.\d+)", out).group(1)

    def netem(self, spec: str, dst: str | None = None, port: int | None = None):
        """Shape what leaves this container (`spec` as `tc qdisc ... netem` takes it, e.g.
        "delay 300ms 50ms loss 2% reorder 25% 50%"). With `dst`, only packets to that IP;
        with `port` too, only those of TCP connections with that port at either end (the
        server's), so the case's own control traffic to a host is left alone."""
        self.clear_netem()
        dev = self.dev
        if dst is None:
            self.sh(f"tc qdisc add dev {dev} root netem {spec}")
        else:
            # A prio qdisc whose third band (unused by the default priomap) gets the
            # filtered packets and holds the netem.
            match = f"match ip dst {dst}/32"
            filters = [f"{match} match ip dport {port} 0xffff", f"{match} match ip sport {port} 0xffff"] \
                if port else [match]
            self.sh(f"tc qdisc add dev {dev} root handle 1: prio && "
                    f"tc qdisc add dev {dev} parent 1:3 handle 30: netem {spec} && " +
                    " && ".join(f"tc filter add dev {dev} protocol ip parent 1:0 prio 3 u32 {f} flowid 1:3"
                                for f in filters))
        self.t._cleanup.append(self.clear_netem)
        print(f"{stamp()} {self.where}: netem {spec}" + (f" to {dst}" if dst else "") + (f" port {port}" if port else ""))

    def clear_netem(self):
        self.sh(f"tc qdisc del dev {self.dev} root 2>/dev/null || true")

    def _ensure_chain(self):
        if not self._chain:
            self.sh(f"iptables -N {self.CHAIN} 2>/dev/null || iptables -F {self.CHAIN}; "
                    f"iptables -C INPUT -j {self.CHAIN} 2>/dev/null || iptables -I INPUT -j {self.CHAIN}; "
                    f"iptables -C OUTPUT -j {self.CHAIN} 2>/dev/null || iptables -I OUTPUT -j {self.CHAIN}")
            self._chain = True
            self.t._cleanup.append(self.restore)

    @staticmethod
    def _rules(peer: str, port: int | None, direction: str, target: str) -> list[str]:
        """iptables matches for TCP to and from the peer's `port` (any traffic with the
        peer without one): "in" what the peer sends, "out" what goes to it."""
        rules = []
        if direction in ("both", "in"):
            rules.append(f"-s {peer}" + (f" -p tcp --sport {port}" if port else "") + f" -j {target}")
        if direction in ("both", "out"):
            rules.append(f"-d {peer}" + (f" -p tcp --dport {port}" if port else "") + f" -j {target}")
        return rules

    def drop(self, peer: str, port: int | None = None, direction: str = "both"):
        """Silently drop packets of the connections to the peer's `port`: "in" from the
        peer, "out" to it, or both. Nothing tells either side: a half-open connection."""
        self._ensure_chain()
        self.sh(" && ".join(f"iptables -A {self.CHAIN} {r}" for r in self._rules(peer, port, direction, "DROP")))
        print(f"{stamp()} {self.where}: DROP {direction} {peer}" + (f":{port}" if port else ""))

    def reject(self, peer: str, port: int):
        """Answer every TCP packet of the connections to the peer's port with a reset."""
        self._ensure_chain()
        self.sh(f"iptables -A {self.CHAIN} -p tcp -d {peer} --dport {port} -j REJECT --reject-with tcp-reset && "
                f"iptables -A {self.CHAIN} -p tcp -s {peer} --sport {port} -j REJECT --reject-with tcp-reset")
        print(f"{stamp()} {self.where}: REJECT (RST) {peer}:{port}")

    def restore(self):
        """Remove every DROP and REJECT: the network is back."""
        if self._chain:
            self.sh(f"iptables -F {self.CHAIN}")
            print(f"{stamp()} {self.where}: iptables rules removed")

    def kill_sockets(self, peer: str, port: int) -> R:
        """Destroy this container's TCP sockets to the peer's port (ss -K): the kernel
        resets them."""
        return self._run("ss", "-K", "dst", peer, "dport", "=", f":{port}", check=False)


# ---------------------------------------------------------------------------- the context


class T:
    """Context of a fixture build or a case: processes, servers, drivers, assertions."""

    def __init__(self, root: Path = FX):
        self.root = root
        self.backend = BACKEND
        self.metrics: dict = {}
        self.soft_failures: list[str] = []
        self._cleanup: list[Callable] = []
        self._servers: list[Server] = []
        self._n = 0
        TMP.mkdir(parents=True, exist_ok=True)

    def need(self, *capabilities: str):
        missing = [c for c in capabilities if c not in CAPABILITIES.get(self.backend, set())]
        if missing:
            raise Blocked(f"backend {self.backend} lacks {', '.join(missing)}")

    def scratch(self, name: str) -> Path:
        self._n += 1
        d = TMP / f"{name}-{self._n}"
        d.mkdir(parents=True, exist_ok=True)
        return d

    def manifest(self) -> dict:
        return json.loads(MANIFEST.read_text())

    # ---- processes

    def run(self, *argv, cwd=None, env=None, input: str | None = None, timeout: float = 120, quiet=False,
            check: bool = False) -> R:
        argv = [str(a) for a in argv]
        full_env = dict(os.environ, **(env or {}))
        try:
            p = subprocess.run(argv, cwd=cwd or self.root, env=full_env, input=input, capture_output=True,
                               text=True, timeout=timeout, stdin=None if input is not None else subprocess.DEVNULL)
            r = R(argv, p.returncode, p.stdout, p.stderr)
        except FileNotFoundError as e:
            r = R(argv, 127, "", f"{argv[0]}: command not found ({e})\n")
        if not quiet:
            print(f"{stamp()} $ {shlex.join(argv)}" + (f"   (cwd {cwd})" if cwd else ""))
            if r.out:
                print(_indent(r.out), end="")
            if r.err:
                print("  stderr:\n" + _indent(r.err), end="")
            print(f"  -> exit {r.code}")
        if check and r.code != 0:
            raise Failure(f"{shlex.join(argv)} exited {r.code}\n{r.text}")
        return r

    def sh(self, script: str, **kw) -> R:
        return self.run("bash", "-c", script, **kw)

    def spawn(self, *argv, env=None, log: Path | None = None, new_session: bool = False) -> subprocess.Popen:
        """A background process, stopped when the case ends."""
        out = open(log, "ab") if log else subprocess.DEVNULL
        p = subprocess.Popen([str(a) for a in argv], env=dict(os.environ, **(env or {})), stdin=subprocess.DEVNULL,
                             stdout=out, stderr=subprocess.STDOUT, start_new_session=new_session)
        self._cleanup.append(lambda: p.poll() is None and p.kill())
        return p

    def proc(self, pid: int) -> Proc | None:
        self.need("procfs")
        return proc(pid)

    def procs(self, pattern: str = "") -> list[Proc]:
        """Live processes whose command line matches the regex."""
        self.need("procfs")
        return [p for p in procs() if p.alive and re.search(pattern, p.cmdline)]

    def listener_pid(self, port: int) -> int:
        """The pid of the process listening on a TCP port."""
        r = self.run("ss", "-Hltnp", f"sport = :{port}", quiet=True)
        m = re.search(r"pid=(\d+)", r.out)
        if not m:
            raise Failure(f"nothing listens on port {port}:\n{r.text}")
        return int(m.group(1))

    # ---- the product

    def server_env(self, dir: Path, state: Path | None = None) -> dict:
        return {
            "TILR_RUNTIME_DIR": str(dir / "runtime"),
            "TILR_STATE_DIR": str(state or dir / "state"),
            "TILR_CONFIG_DIR": str(dir / "config"),
            "HOME": os.environ.get("HOME", "/root"),
            "TERM": "xterm-256color",
            "NO_COLOR": "1",
        }

    def install(self, exe: str = SERVER_EXE, name: str = "tilr-server") -> Path:
        """A copy of the server in a directory of its own, where an update may replace it."""
        dest = self.scratch("install") / name
        shutil.copy2(shutil.which(exe) or exe, dest)
        return dest

    def server(self, *args, name: str = "tilrd", exe: str | Path = SERVER_EXE, listen: str = "127.0.0.1:0",
               state: Path | None = None, env: dict | None = None) -> Server:
        """Start tilrd as a child of the case, with directories of its own; ready to use."""
        dir = self.scratch(name)
        senv = {**self.server_env(dir, state), **(env or {})}
        log = dir / "log"
        argv = [str(exe), "--listen", listen, *map(str, args)]
        print(f"{stamp()} start {name}: {shlex.join(argv)}")
        p = subprocess.Popen(argv, env=dict(os.environ, **senv), stdin=subprocess.DEVNULL,
                             stdout=open(log, "ab"), stderr=subprocess.STDOUT)
        s = Server(self, name, str(exe), dir, senv, log, p)
        self._servers.append(s)
        return s.wait_ready()

    def found_server(self, log: Path, dir: Path, env: dict, name: str = "tilrd", exe: str = SERVER_EXE,
                     timeout: float = 30) -> Server:
        """A tilrd started some other way (from a terminal session); found by its log."""
        s = Server(self, name, exe, dir, env, log)
        self._servers.append(s)
        return s.wait_ready(timeout)

    def driver(self, name: str = "driver") -> Driver:
        d = Driver(self, name)
        self._cleanup.append(d.quit)
        return d

    def session(self, *argv, env: dict | None = None, name: str = "ssh") -> Session:
        """A terminal login session running argv (an interactive shell, typically)."""
        self.need("linux")
        s = Session(self, list(argv), dict(os.environ, **(env or {})), name)
        self._cleanup.append(lambda: s.p.poll() is None and s.p.kill())
        return s

    def proxy(self, target_port: int, target_host: str = "127.0.0.1"):
        """A layer-1 fault proxy in front of target (see tt/proxy.py)."""
        from .proxy import Proxy
        p = Proxy(target_host, target_port)
        self._cleanup.append(p.close)
        print(f"{stamp()} proxy 127.0.0.1:{p.port} -> {target_host}:{target_port}")
        return p

    def host(self, name: str) -> Host:
        """A host the case declared in @case(hosts=...), once its agent answers."""
        self.need("hosts")
        addr = os.environ.get(f"TT_HOST_{name.upper()}")
        if not addr:
            raise Blocked(f"host {name} is not available on backend {self.backend}")
        h = Host(self, name, addr)
        print(f"{stamp()} host {name} at {h.ip} (sees this container as {h.peer_ip})")
        return h

    def net(self) -> Net:
        """Layer 2 faults on this container's own network (needs @case(net_admin=True))."""
        self.need("netem")
        if os.environ.get("TT_NET_ADMIN") != "1":
            raise Blocked("the case's container has no NET_ADMIN: declare @case(net_admin=True)")
        return Net(self, lambda *argv, **kw: self.run(*argv, **{"quiet": True, **kw}), "case")

    def proxy_service(self, name: str = "proxy"):
        """The fault proxy running in a service container of its own (services=["proxy"]);
        point it somewhere with .target(host, port)."""
        from .proxy import RemoteProxy
        addr = self.service(name)
        p = RemoteProxy(addr)
        self._cleanup.append(p.close)
        return p

    def service(self, name: str) -> str:
        """URL of a service the case declared in @case(services=...)."""
        url = os.environ.get(f"TT_SERVICE_{name.upper()}")
        if not url:
            raise Blocked(f"service {name} is not available on backend {self.backend}")
        return url

    # ---- results

    def metric(self, name: str, value):
        """A measurement recorded in the result (report.json, summary)."""
        self.metrics[name] = value
        print(f"  metric {name} = {value}")
        try:
            METRICS.write_text(json.dumps(self.metrics))
        except OSError:
            pass

    def cleanup(self):
        for fn in reversed(self._cleanup):
            try:
                fn()
            except Exception:
                pass
        for s in self._servers:
            s.stop()

    def dump_logs(self):
        for s in self._servers:
            text = s.log_text()
            if text:
                print(f"--- {s.name} log ({s.log}) ---\n{_indent(text, 80)}")
        for f in sorted(TMP.glob("*.stderr")):
            text = f.read_text(errors="replace").strip()
            if text:
                print(f"--- {f.name} ---\n{_indent(text, 40)}")

    # ---- waiting and assertions

    def eventually(self, fn: Callable, what: "str | Callable[[], str]", timeout: float = 10, interval: float = 0.1):
        """Poll fn until it returns something truthy; that value. `what` may be a function
        that describes the failure when it happens."""
        deadline = time.monotonic() + timeout
        while True:
            value = fn()
            if value:
                return value
            if time.monotonic() >= deadline:
                raise Failure(f"did not happen within {timeout}s: {what() if callable(what) else what}")
            time.sleep(interval)

    def eq(self, actual, expected, what: str = "value"):
        if actual != expected:
            raise Failure(f"{what}: expected {expected!r}, got {actual!r}")
        print(f"  ok: {what} == {expected!r}")

    def ok(self, cond, what: str):
        """Assert `what` (a statement of what must hold) is true."""
        if not cond:
            raise Failure(f"not so: {what}")
        print(f"  ok: {what}")

    def has(self, text: str, *needles: str):
        for n in needles:
            if n not in text:
                raise Failure(f"expected {n!r} in:\n{text}")
            print(f"  ok: contains {n!r}")

    def lacks(self, text: str, *needles: str):
        for n in needles:
            if n in text:
                raise Failure(f"unexpected {n!r} in:\n{text}")
            print(f"  ok: lacks {n!r}")

    def check(self, what: str, fn: Callable):
        """A soft check: run fn (assertions); a failure is recorded and the case goes on.
        The case fails at the end if any check did."""
        try:
            fn()
        except Blocked:
            raise
        except Exception as e:
            self.soft_failures.append(f"{what}: {e}")
            print(f"  FAILED check: {what}: {e}")
        else:
            print(f"  ok: {what}")
