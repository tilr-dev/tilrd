"""UC-077: Exec over the protocol, without a shell or a pane: a client runs a program on the
server's machine and gets its stdout and stderr apart, its exit code or the signal that
killed it; a time limit, cancelling, a large output, stdin, several at once. [no feature]

tilrd has no Exec yet: tilr.proto has no such messages and HelloAck.features no "exec".
The cases are written against the driver's draft of the interface (crates/tilr-driver/src/
exec.rs: ExecStart {argv, cwd, env, stdin, timeout_ms} -> ExecStarted, ExecOutput per
stream, ExecExit {code, signal, timed_out, cancelled}; ExecInput, ExecSignal,
ExecCancel), which the driver sends only to a server that offers "exec". Until then
every case fails at its first `exec` with "the server does not offer exec". When the
real messages land, the draft goes and the cases stay.
"""

import hashlib
import os
import time

from tt import T, case


def start(t: T):
    server = t.server(state=t.root / "state")
    d = t.driver()
    hello = d.connect(server, features=["ping", "exec"])
    t.ok("exec" in hello["features"], f"the server offers exec (HelloAck.features {hello['features']})")
    return server, d


@case(fixture="server_state")
def exec_returns_stdout_and_stderr_apart_and_the_exit_code(t: T):
    """UC-077: sh -c 'echo out; echo err >&2; exit 3': stdout, stderr apart, code 3."""
    server, d = start(t)
    r = d.cmd("exec", argv=["sh", "-c", "echo out; echo err >&2; exit 3"])
    t.eq((r["stdout"], r["stderr"], r["code"], r["signal"]), ("out\n", "err\n", 3, 0), "outputs and status")


@case(fixture="server_state")
def exec_runs_no_shell_so_nothing_is_expanded(t: T):
    """UC-077: argv goes to the program as it is: no globbing, variables, substitution; cwd and env as asked."""
    server, d = start(t)
    r = d.cmd("exec", argv=["printf", "%s|", "$HOME", "*", "`id`", "a b"], cwd="/tmp", env=["TT_X=1"])
    t.eq((r["stdout"], r["code"]), ("$HOME|*|`id`|a b|", 0), "the arguments as given")
    r = d.cmd("exec", argv=["sh", "-c", "pwd; echo $TT_X"], cwd="/tmp", env=["TT_X=from-exec"])
    t.eq(r["stdout"], "/tmp\nfrom-exec\n", "cwd and env")
    r = d.cmd("exec", argv=["no-such-program-tt"], ok=False)
    t.eq(r["ev"], "error", "a program that does not exist is refused at start")


@case(fixture="server_state")
def exec_reports_the_signal_that_killed_the_program(t: T):
    """UC-077: a program killed by SIGTERM (its own) and one sent SIGINT by the client: signal reported."""
    server, d = start(t)
    r = d.cmd("exec", argv=["sh", "-c", "kill -TERM $$"])
    t.eq((r["signal"], r["timed_out"], r["cancelled"]), (15, False, False), "killed by SIGTERM")
    started = d.cmd("exec", argv=["sleep", "30"], background=True)
    time.sleep(0.5)
    d.cmd("exec_signal", exec=started["exec"], signal=2)
    r = d.cmd("exec_wait", exec=started["exec"], timeout_ms=5000)
    t.eq(r["signal"], 2, "killed by the SIGINT the client sent")


@case(fixture="server_state")
def exec_with_a_time_limit_is_killed_and_leaves_nothing_behind(t: T):
    """UC-077: sleep 30 with a 1 s limit: timed out within 3 s, the process and its children gone."""
    server, d = start(t)
    began = time.monotonic()
    r = d.cmd("exec", argv=["sh", "-c", "sleep 30 & sleep 31; wait"], limit_ms=1000, timeout_ms=10_000)
    t.ok(r["timed_out"], "timed out")
    t.ok(time.monotonic() - began < 3, "within 3 s")
    t.eventually(lambda: not t.procs(r"^sleep 3[01]$"), "no sleep is left running", timeout=3)


@case(fixture="server_state")
def a_cancelled_exec_ends_at_once_with_its_children(t: T):
    """UC-077: ExecCancel on a running program: it and its children end, the exit says cancelled."""
    server, d = start(t)
    started = d.cmd("exec", argv=["sh", "-c", "sleep 40 & sleep 41; wait"], background=True)
    t.eventually(lambda: t.procs(r"^sleep 4[01]$"), "the program runs", timeout=5)
    d.cmd("exec_cancel", exec=started["exec"])
    r = d.cmd("exec_wait", exec=started["exec"], timeout_ms=5000)
    t.ok(r["cancelled"], "the exit says cancelled")
    t.eventually(lambda: not t.procs(r"^sleep 4[01]$"), "no sleep is left running", timeout=3)


@case(fixture="server_state")
def a_large_output_arrives_whole_and_in_order(t: T):
    """UC-077: 64 MiB on stdout and 1 MiB on stderr interleaved: both whole (sha256), apart."""
    server, d = start(t)
    data = t.scratch("exec") / "data.bin"
    data.write_bytes(os.urandom(64 << 20))
    err = t.scratch("exec") / "err.bin"
    err.write_bytes(os.urandom(1 << 20))
    r = d.cmd("exec", argv=["sh", "-c", f"cat {data} & cat {err} >&2; wait"], timeout_ms=120_000)
    t.eq(r["code"], 0, "exit status")
    t.eq((r["stdout_bytes"], r["stdout_sha256"]), (64 << 20, hashlib.sha256(data.read_bytes()).hexdigest()),
         "stdout whole")
    t.eq((r["stderr_bytes"], r["stderr_sha256"]), (1 << 20, hashlib.sha256(err.read_bytes()).hexdigest()),
         "stderr whole")


@case(fixture="server_state")
def stdin_given_to_exec_reaches_the_program(t: T):
    """UC-077: sha256sum fed 10 MiB on stdin in pieces, then EOF: the hash of what was sent."""
    server, d = start(t)
    data = t.scratch("exec") / "in.bin"
    data.write_bytes(os.urandom(10 << 20))
    started = d.cmd("exec", argv=["sha256sum"], stdin="", background=True)
    t.eq(d.cmd("exec_wait", exec=started["exec"], timeout_ms=3000)["stdout"],
         f"{hashlib.sha256(b'').hexdigest()}  -\n", "an empty stdin, closed, is read as such")
    started = d.cmd("exec", argv=["sha256sum"], stdin=None, background=True)
    d.cmd("exec_input", exec=started["exec"], **{"from": str(data)})
    d.cmd("exec_input", exec=started["exec"], eof=True)
    r = d.cmd("exec_wait", exec=started["exec"], timeout_ms=30_000)
    t.eq(r["stdout"], f"{hashlib.sha256(data.read_bytes()).hexdigest()}  -\n", "the program read all of it")


@case(fixture="server_state")
def eight_execs_run_at_once(t: T):
    """UC-077: eight programs of 1 s each started together end in about 1 s, each with its own output."""
    server, d = start(t)
    began = time.monotonic()
    runs = [d.cmd("exec", argv=["sh", "-c", f"sleep 1; echo run {n}"], background=True)["exec"] for n in range(8)]
    outs = [d.cmd("exec_wait", exec=e, timeout_ms=10_000) for e in runs]
    t.eq([o["stdout"] for o in outs], [f"run {n}\n" for n in range(8)], "each its own output")
    t.ok(time.monotonic() - began < 4, f"they ran at once ({time.monotonic() - began:.1f} s)")
    t.eq(d.run(d.shell_pane(), "echo panes-still-work"), 0, "panes work beside them")
