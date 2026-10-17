"""UC-065: the disk runs out of space in the middle of an update: while the new executable
is downloaded next to the old one, and while the server writes the state it hands its
successor at a reload. Nothing breaks: the installed executable stays as it was with
nothing left beside it, the server keeps its pid and its panes, shells keep their state,
no half-written state file is left behind, and once there is room again the update and
the reload go through.

The small disk is a tmpfs (mount needs the root capabilities of net_admin=True): one the
size of the executable plus a little for its directory, one of a few MB for the
server's runtime directory (TILR_RUNTIME_DIR: reload state, pane state, sockets, the
clipboard), filled up with a file of zeros.

Was red until 2026-10-17: a reload that could not write its state left the half-written
`server-<pid>.state` or `panes/<id>.state` behind. Now a state file is written to a
temporary file next to it, synced and renamed, and the temporary file is removed on any
error (crates/tilr-server/src/reload.rs `write_file`).
"""

import hashlib
import os
import signal
import time
from pathlib import Path

from tt import SERVER_EXE, T, Counter, case
from tt.live import state_files, wait_reloads


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fill(t: T, dir: Path, leave: int = 0):
    """Fill the file system of `dir` up, leaving `leave` bytes (whole 4 KiB pages)."""
    reserve = dir / "tt-reserve"
    if leave:
        reserve.write_bytes(b"\0" * leave)
    t.sh(f"dd if=/dev/zero of={dir}/tt-filler bs=4k 2>/dev/null; sync; df -k {dir}", quiet=False)
    if leave:
        reserve.unlink()


def panes(t: T, d):
    shell = d.shell_pane()
    d.line(shell, "export MARK=kept; sleep 9999 &")
    before = d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)")
    counter_pane = d.shell_pane()
    counter = Counter(t, d, counter_pane, period=0.2)
    d.wait_for(counter_pane, r"(?m)^tick 3$")
    return shell, before, counter


def all_well(t: T, server, d, shell, before, counter, pids, what: str):
    t.check(f"the server keeps its pid {what}", lambda: (t.ok(server.alive(), "alive"),
                                                        t.eq(t.listener_pid(server.port), server.pid, "listener")))
    t.check(f"the pane processes are the same {what}",
            lambda: t.eq(sorted(p.pid for p in server.panes()), pids, "pane pids"))
    t.check(f"the shell keeps its variable, pid and job {what}",
            lambda: t.eq(d.value(shell, "$MARK $$ $(jobs -r | grep -c sleep)"), before, "shell"))
    t.check(f"the counter goes on {what}", lambda: counter.check_screen(d, what=f"the screen {what}"))

    def a_new_client_works():
        other = t.driver(f"other-{time.monotonic_ns()}")
        other.connect(server)
        pane = other.shell_pane()
        t.eq(other.run(pane, "echo fresh"), 0, "a command in a new pane")
        other.cmd("close_pane", pane=pane)
        t.eventually(lambda: len(server.panes()) == len(pids), "the new pane is gone again", timeout=10)
        other.quit()
    t.check(f"a new client and a new pane work {what}", a_new_client_works)


@case(fixture="server_state", services=["cdn"], net_admin=True, timeout=300)
def an_update_that_runs_out_of_disk_while_downloading_installs_nothing_and_breaks_nothing(t: T):
    """UC-065: the download does not fit the executable's disk: update fails, nothing installed or left; with room it works."""
    manifest = t.manifest()
    size = Path(os.path.realpath(t.run("which", SERVER_EXE, quiet=True).out.strip())).stat().st_size
    disk = t.tmpfs(t.root / "small-disk", f"{size // 1024 + 512}k")
    exe = disk / "tilr-server"
    exe.write_bytes(Path(os.path.realpath(t.run("which", SERVER_EXE, quiet=True).out.strip())).read_bytes())
    exe.chmod(0o755)
    installed = digest(exe)
    server = t.server(exe=exe, state=t.root / "state")
    d = t.driver()
    d.connect(server)
    shell, before, counter = panes(t, d)
    pids = sorted(p.pid for p in server.panes())

    r = server.cmd("update", "--cdn", t.service("cdn"), timeout=180)
    t.ok(r.code != 0, f"the update fails (exit {r.code})")
    t.ok("space" in r.text.lower(), f"and says the disk is full: {r.text.strip()[-300:]}")
    t.eq(digest(exe), installed, "the installed executable is as it was")
    t.eq(sorted(p.name for p in disk.iterdir()), ["tilr-server"], "nothing is left beside it")
    time.sleep(3)
    t.eq(server.logged("reloading"), 0, "the server did not reload")
    all_well(t, server, d, shell, before, counter, pids, "after the failed update")

    t.run("mount", "-o", f"remount,size={2 * size // 1024 + 1024}k", disk, check=True)
    since = d.mark()
    r = server.cmd("update", "--cdn", t.service("cdn"), timeout=180)
    t.eq(r.code, 0, "with room, the update succeeds")
    wait_reloads(t, server, 1)
    d.wait_event("disconnected", since=since, timeout=10)
    back = d.cmd("reconnect", timeout_ms=20_000)
    t.eq(back["server_version"], manifest["next_version"], "the server runs the next version")
    all_well(t, server, d, shell, before, counter, pids, "after the update with room")
    counter.check_log()


def runtime_on_a_small_disk(t: T):
    runtime = t.tmpfs(t.root / "runtime", "4m")
    server = t.server(state=t.root / "state", env={"TILR_RUNTIME_DIR": str(runtime)})
    d = t.driver()
    d.connect(server)
    shell, before, counter = panes(t, d)
    vim = d.shell_pane()
    notes = t.scratch("vim") / "notes.txt"
    d.line(vim, f"vim -n -u NONE -N {notes}")
    d.wait_for(vim, r'"\S*notes\.txt" \[New\]', timeout=15)
    d.input(vim, "iunsaved text\x1b")
    d.wait_for(vim, r"(?m)^unsaved text$")
    pids = sorted(p.pid for p in server.panes())
    return runtime, server, d, shell, before, counter, vim, notes, pids


def room_again_and_reload(t: T, runtime, server, d, shell, before, counter, vim, notes, pids, reloads: int):
    (runtime / "tt-filler").unlink()
    since = d.mark()
    server.signal(signal.SIGUSR1)
    wait_reloads(t, server, reloads + 1)
    d.wait_event("disconnected", since=since, timeout=10)
    d.cmd("reconnect", timeout_ms=20_000)
    all_well(t, server, d, shell, before, counter, pids, "after the reload with room")

    def vim_writes():
        d.input(vim, "\x1b:wq\r")
        d.wait_for(vim, r"(?m)^\$ ?$")
        t.eq(notes.read_text(), "unsaved text\n", "what :wq wrote")
    t.check("vim kept its buffer through it all", vim_writes)
    counter.check_log(max_gap=1.5)


@case(fixture="server_state", net_admin=True)
def a_reload_with_no_room_for_its_state_keeps_the_server_and_its_panes(t: T):
    """UC-065: the runtime directory's disk is full; SIGUSR1 cannot write the state: the server carries on, panes kept."""
    runtime, server, d, shell, before, counter, vim, notes, pids = runtime_on_a_small_disk(t)
    fill(t, runtime)
    server.signal(signal.SIGUSR1)
    t.eventually(lambda: server.logged("reload failed") or server.logged("not reloading")
                 or server.logged("resumed after reload"), "the server tries to reload", timeout=15)
    time.sleep(2)
    t.check("the reload did not happen", lambda: t.eq(server.logged("resumed after reload with"), 0, "reloads"))
    t.check("the client was not dropped", lambda: t.ok(d.connected(), "still connected"))
    all_well(t, server, d, shell, before, counter, pids, "after the reload that had no room")
    t.check("no half-written state file is left", lambda: t.eq(state_files(server), [], "state files"))
    room_again_and_reload(t, runtime, server, d, shell, before, counter, vim, notes, pids, reloads=0)


@case(fixture="server_state", net_admin=True)
def a_reload_with_room_for_the_servers_state_but_not_the_panes_keeps_every_pane(t: T):
    """UC-065: room for the supervisor's state only (one page): the panes cannot save theirs; every pane lives on."""
    runtime, server, d, shell, before, counter, vim, notes, pids = runtime_on_a_small_disk(t)
    fill(t, runtime, leave=4096)
    since = d.mark()
    server.signal(signal.SIGUSR1)
    t.eventually(lambda: server.logged("reload failed") or server.logged("not reloading")
                 or server.logged("resumed after reload"), "the server tries to reload", timeout=15)
    time.sleep(3)
    if any(e.get("ev") == "disconnected" for e in d.events[since:]):
        d.cmd("reconnect", timeout_ms=20_000)
    t.metric("panes_reloaded", server.logged("tilr_server::pane: resumed after reload"))
    all_well(t, server, d, shell, before, counter, pids, "after the reload with no room for the panes")
    t.check("no half-written state file is left", lambda: t.eq(state_files(server), [], "state files"))
    room_again_and_reload(t, runtime, server, d, shell, before, counter, vim, notes, pids,
                          reloads=server.logged("resumed after reload with"))
