"""Live software in panes, for the cases that update or reload the server (group F).

    live = LivePanes(t, d, server, echo=t.service("echo"))   # every program, started
    ... update / reload, the client back ...
    live.check("the update")      # soft checks that change nothing: alive, same screens
    live.finish()                 # soft checks that use the programs up: :wq, q, Ctrl-C

The panes: a shell with a variable and a background job, a counter, vim with an unsaved
buffer and the mouse on, htop, less on /fx/big.txt (fixture `files`), a python REPL
holding x = 42, a foreground `sleep 1000`, and a program keeping a TCP connection to the
echo service (`services=["echo"]`) busy. `which` picks some of them.
"""

from __future__ import annotations

import re
import time
from pathlib import Path

from . import T, Counter, Driver, Server, descendants, proc

ALL = ("shell", "counter", "vim", "htop", "less", "python", "sleep", "tcp")

# Talks to an echo service over one TCP connection: a numbered line every 0.2 s, each
# checked when it comes back; `connected <local port>` once, `<n> ok|BAD <time>` per
# line, `EOF` / an error if the connection ends.
ECHO_CLIENT = r"""
import socket, sys, time
host, port, log = sys.argv[1], int(sys.argv[2]), open(sys.argv[3], "a", buffering=1)
s = socket.create_connection((host, port), timeout=60)
log.write(f"connected {s.getsockname()[1]}\n")
n = 0
while True:
    n += 1
    msg = f"{n:08d}\n".encode()
    try:
        s.sendall(msg)
        got = b""
        while len(got) < len(msg):
            chunk = s.recv(len(msg) - len(got))
            if not chunk:
                log.write("EOF\n")
                sys.exit(1)
            got += chunk
    except OSError as e:
        log.write(f"ERROR {e}\n")
        sys.exit(1)
    log.write(f"{n} {'ok' if got == msg else 'BAD'} {time.time():.3f}\n")
    time.sleep(0.2)
"""

# Long-lived processes of the programs, by what their command line is.
LONG_LIVED = r"^((\S*/)?-?bash|vim |htop|less |python3|sleep (1000|9999)$|.*(tilrd|tilr-server).* pane --id )"


class LivePanes:
    def __init__(self, t: T, d: Driver, server: Server, echo: str | None = None,
                 which: tuple[str, ...] = ALL):
        self.t, self.d, self.server = t, d, server
        self.which = which
        self.panes: dict[str, str] = {}
        self.before: dict[str, dict] = {}
        self.dir = t.scratch("live")
        if "shell" in which:
            p = self.panes["shell"] = d.shell_pane()
            d.line(p, "export MARK=survived; sleep 9999 &")
            self.shell_value = d.value(p, "$MARK $$ $(jobs -r | grep -c sleep)")
            t.eq(self.shell_value.split()[::2], ["survived", "1"], "the shell has its variable and its job")
        if "counter" in which:
            p = self.panes["counter"] = d.shell_pane()
            self.counter = Counter(t, d, p, period=0.2)
            d.wait_for(p, r"(?m)^tick 5$")
        if "vim" in which:
            p = self.panes["vim"] = d.shell_pane()
            self.notes = self.dir / "todo.txt"
            d.line(p, f"vim -n -u NONE -N {self.notes}")
            d.wait_for(p, r'"\S*todo\.txt" \[New\]', timeout=15)
            d.input(p, ":set mouse=a\r")
            d.input(p, "ihello from before the reload\x1b")
            d.wait_for(p, r"(?m)^hello from before the reload$")
            t.eventually(lambda: d.screen(p)["mouse_tracking"], "vim turns mouse tracking on")
            # The screen to compare is the one after Esc took vim out of insert mode.
            t.eventually(lambda: "-- INSERT --" not in d.text(p), "vim leaves insert mode")
        if "htop" in which:
            p = self.panes["htop"] = d.shell_pane()
            d.line(p, "htop")
            d.wait_for(p, r"Load average", timeout=15)
        if "less" in which:
            p = self.panes["less"] = d.shell_pane()
            d.line(p, f"less {t.root / 'big.txt'}")
            d.wait_for(p, r"(?m)^line 1$")
            d.key(p, "G")
            d.wait_for(p, r"(?m)^line 200000$")
        if "python" in which:
            p = self.panes["python"] = d.shell_pane()
            d.line(p, "python3 -q")
            d.wait_for(p, r"(?m)^>>> ?$")
            d.line(p, "x = 42")
            d.line(p, "print('x is', x)")
            d.wait_for(p, r"(?m)^x is 42$")
        if "sleep" in which:
            p = self.panes["sleep"] = d.shell_pane()
            d.line(p, "sleep 1000")
            t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep 1000 runs")
            self.sleep_pid = t.procs(r"^sleep 1000$")[0].pid
        if "tcp" in which:
            if not echo:
                raise ValueError("the tcp program needs the echo service")
            host, port = echo.removeprefix("tcp://").rsplit(":", 1)
            program = self.dir / "echo_client.py"
            program.write_text(ECHO_CLIENT)
            self.tcp_log = self.dir / "echo.log"
            p = self.panes["tcp"] = d.shell_pane()
            d.line(p, f"python3 {program} {host} {port} {self.tcp_log}")
            t.eventually(lambda: self.tcp_lines(ok_only=True) >= 3, "the program talks to the echo service",
                         timeout=15)
        self.pane_ids = sorted(p["pane"] for p in d.list())
        self.pane_pids = sorted(p.pid for p in server.panes())
        self.long_lived = self.processes()
        self.fds = self.descriptors()
        for name in ("vim", "htop", "less"):
            if name in which:
                self.before[name] = self.modes(name)

    # ---- what there is to compare

    def tcp_lines(self, ok_only: bool = False) -> int:
        if not self.tcp_log.exists():
            return 0
        lines = self.tcp_log.read_text().splitlines()
        return sum(1 for l in lines if " ok " in l) if ok_only else len(lines)

    def processes(self) -> dict[int, str]:
        """The long-lived processes under the server: pid -> command line."""
        return {p.pid: p.cmdline for p in descendants(self.server.pid)
                if p.alive and re.search(LONG_LIVED, p.cmdline)}

    def descriptors(self) -> dict[int, int]:
        """Open descriptors of the supervisor and of each pane process."""
        out = {}
        for pid in [self.server.pid, *self.pane_pids]:
            p = proc(pid)
            if p:
                out[pid] = p.fds
        return out

    def modes(self, name: str) -> dict:
        s = self.d.screen(self.panes[name])
        return {"text": s["text"], "alt_screen": s["alt_screen"], "mouse_tracking": s["mouse_tracking"],
                "cursor_visible": s["cursor_visible"]}

    # ---- checks

    def check(self, what: str = "the reload"):
        """Soft checks that use nothing up: the same panes and processes, the programs
        alive, their screens and modes as they were."""
        t, d = self.t, self.d
        t.check(f"the server keeps its pid after {what}", lambda: t.ok(self.server.alive(), "alive"))
        t.check(f"the panes are the same after {what}",
                lambda: t.eq(sorted(p["pane"] for p in d.list()), self.pane_ids, "pane ids"))
        t.check(f"the pane processes are the same after {what}",
                lambda: t.eq(sorted(p.pid for p in self.server.panes()), self.pane_pids, "pids"))

        def every_program_lives():
            now = self.processes()
            gone = {pid: cmd for pid, cmd in self.long_lived.items() if pid not in now}
            t.eq(gone, {}, "long-lived processes gone")
            new = {pid: cmd for pid, cmd in now.items() if pid not in self.long_lived}
            t.eq(new, {}, "long-lived processes that were not there before")
        t.check(f"every program keeps its process after {what}", every_program_lives)

        def no_orphans():
            mine = {p.pid for p in descendants(self.server.pid)}
            strays = [f"{p.pid} {p.cmdline}" for p in t.procs(r"(tilrd|tilr-server).* pane --id |^vim |^htop|^less ")
                      if p.pid not in mine]
            t.eq(strays, [], "pane processes and programs outside the server's tree")
        t.check(f"no orphans after {what}", no_orphans)

        if "shell" in self.which:
            t.check(f"the shell keeps its variable, pid and job after {what}",
                    lambda: t.eq(d.value(self.panes["shell"], "$MARK $$ $(jobs -r | grep -c sleep)"),
                                 self.shell_value, "shell"))
        if "counter" in self.which:
            t.check(f"the counter goes on after {what}",
                    lambda: self.counter.check_screen(d, what=f"the screen after {what}"))
        for name in ("vim", "less"):
            if name in self.which:
                t.check(f"{name} shows the same screen in the same modes after {what}",
                        lambda name=name: t.eq(self.modes(name), self.before[name], f"{name}'s screen and modes"))
        if "htop" in self.which:
            def htop_same_modes():
                first = d.screen(self.panes["htop"])
                now = {k: first[k] for k in ("alt_screen", "mouse_tracking", "cursor_visible")}
                was = {k: self.before["htop"][k] for k in now}
                t.eq(now, was, "htop's modes")
                t.ok(was["alt_screen"], "htop runs on the alternate screen")
                t.has(first["text"], "Load average")
                d.cmd("sleep", ms=3000)
                later = d.screen(self.panes["htop"])
                t.ok(later["frames"] > first["frames"], f"htop redraws ({first['frames']} -> {later['frames']})")
            t.check(f"htop keeps its modes and redraws after {what}", htop_same_modes)
        if "python" in self.which:
            def python_remembers():
                self.n_python = getattr(self, "n_python", 0) + 1
                d.line(self.panes["python"], f"print('x', {self.n_python}, x)")
                d.wait_for(self.panes["python"], rf"(?m)^x {self.n_python} 42$")
            t.check(f"the python REPL keeps its state after {what}", python_remembers)
        if "sleep" in self.which:
            t.check(f"the foreground sleep runs on after {what}",
                    lambda: t.ok(proc(self.sleep_pid) and proc(self.sleep_pid).alive, "sleep 1000 alive"))
        if "tcp" in self.which:
            def tcp_goes_on():
                n = self.tcp_lines(ok_only=True)
                t.eventually(lambda: self.tcp_lines(ok_only=True) >= n + 5, "the echo exchange goes on", timeout=10)
                lines = self.tcp_log.read_text().splitlines()
                t.eq(len([l for l in lines if l.startswith("connected")]), 1, "one connection, never made again")
                t.eq([l for l in lines if not l.startswith("connected") and " ok " not in l], [],
                     "no bad echo, end or error")
                stamps = [float(l.split()[2]) for l in lines if " ok " in l]
                gap = max(b - a for a, b in zip(stamps, stamps[1:]))
                t.metric("tcp_max_gap_ms", round(gap * 1000))
                t.ok(gap < 2.0, f"the exchange never stalled over 2 s (longest {gap:.2f}s)")
            t.check(f"the outbound TCP connection keeps exchanging after {what}", tcp_goes_on)

    def finish(self):
        """Soft checks that end the programs: vim writes its buffer, htop, less and the REPL
        quit, Ctrl+C stops the sleep, the counter's log has no gap."""
        t, d = self.t, self.d
        if "counter" in self.which:
            t.check("the counter skipped nothing and never stalled", lambda: self.counter.check_log(max_gap=1.0))
        if "vim" in self.which:
            def vim_writes():
                d.input(self.panes["vim"], "\x1b:wq\r")
                d.wait_for(self.panes["vim"], r"(?m)^\$ ?$")
                t.eq(self.notes.read_text(), "hello from before the reload\n", "what :wq wrote")
            t.check("vim keeps its unsaved buffer and :wq writes it", vim_writes)
        if "htop" in self.which:
            def htop_quits():
                d.key(self.panes["htop"], "q")
                d.wait_for(self.panes["htop"], r"(?m)^\$ ?$")
            t.check("htop quits on q", htop_quits)
        if "less" in self.which:
            def less_answers():
                d.key(self.panes["less"], "g")
                d.wait_for(self.panes["less"], r"^line 1\n")
                d.key(self.panes["less"], "q")
                d.wait_for(self.panes["less"], r"(?m)^\$ ?$")
            t.check("less answers keys and quits", less_answers)
        if "python" in self.which:
            def python_exits():
                d.line(self.panes["python"], "exit()")
                d.wait_for(self.panes["python"], r"(?m)^\$ ?$")
            t.check("the python REPL exits", python_exits)
        if "sleep" in self.which:
            def ctrl_c_stops_sleep():
                d.key(self.panes["sleep"], "ctrl-c")
                t.eq(d.value(self.panes["sleep"], "$?"), "130", "sleep's exit status (SIGINT)")
                t.eventually(lambda: not t.procs(r"^sleep 1000$"), "sleep 1000 is gone", timeout=5)
            t.check("Ctrl+C stops the sleep", ctrl_c_stops_sleep)
        if "tcp" in self.which:
            def tcp_stops():
                d.key(self.panes["tcp"], "ctrl-c")
                d.wait_for(self.panes["tcp"], r"(?m)^\$ ?$")
            t.check("the TCP program stops on Ctrl+C", tcp_stops)

    def wait_settled(self, timeout: float = 30):
        """After the client is back: every pane has its full frame again."""
        for p in self.panes.values():
            self.d.cmd("screen", pane=p, quiet=True, timeout_ms=int(timeout * 1000))


def wait_reloads(t: T, server: Server, count: int, timeout: float = 30) -> float:
    """Wait until the server's log says it resumed after `count` reloads in all; seconds."""
    started = time.monotonic()
    t.eventually(lambda: server.logged("resumed after reload with") >= count,
                 f"the server reloads ({count})", timeout=timeout, interval=0.05)
    return time.monotonic() - started


def state_files(server: Server) -> list[str]:
    """Reload state files left in the server's runtime directory."""
    runtime = Path(server.env["TILR_RUNTIME_DIR"])
    return sorted(str(p.relative_to(runtime)) for p in runtime.rglob("*.state"))


def reload_times(server: Server) -> list[float]:
    """Milliseconds each reload took, from the log: the supervisor's `reloading` to the
    last pane's `resumed after reload` before the next one."""
    import datetime
    spans, start, end = [], None, None
    for m in re.finditer(r"(?m)^(\S+Z)\s+INFO (?:tilrd|tilr_server)::(supervisor|pane): (reloading|resumed after reload)",
                         server.log_text()):
        at = datetime.datetime.fromisoformat(m.group(1).replace("Z", "+00:00")).timestamp()
        if m.group(2) == "supervisor" and m.group(3) == "reloading":
            if start is not None and end is not None:
                spans.append(round((end - start) * 1000, 1))
            start, end = at, None
        elif m.group(3) == "resumed after reload" and start is not None:
            end = at
    if start is not None and end is not None:
        spans.append(round((end - start) * 1000, 1))
    return spans
