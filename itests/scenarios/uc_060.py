"""UC-060: `tilrd update` from a fake CDN (a service container) while the panes run
live software: a shell with a variable and a background job, a counter, vim with an
unsaved buffer and the mouse on, htop, less on a big file, a python REPL holding x = 42,
a foreground `sleep 1000`, and a program keeping a TCP connection to an echo service (a
container of its own) busy. The version changes, the server keeps its pid, and every
program is alive, answers, and looks as it did (screens, alternate screen, mouse
tracking, hidden cursor); nothing is left orphaned and no descriptor leaks.

The CDN serves the layout of crates/tilrd/src/update.rs; its release is an archive of
the same source built with another version string (/opt/tt/tilrd-next and tilr-next). The update runs
outside the panes, as `scripts/update-test.sh` runs it on a real host. Each outcome is a
separate check (tt/live.py), so one broken program does not hide the others.
"""

import os
import re
import time

from tt import T, case, strace, syscalls
from tt.live import LivePanes, reload_times

RELOADED = "resumed after reload with"


@case(fixture="server_state", services=["cdn", "echo"], timeout=420, spec=[
    "reload/a-reload-keeps-pids-and-command-lines-and-leaves-no-state-file",
    "reload/inherited-processes-are-waited-without-leaks",
    "update/live-programs-survive-an-update",
])
def update_from_the_cdn_keeps_every_program_in_the_panes_alive(t: T):
    """UC-060: update with a shell, counter, vim, htop, less, python, sleep and a TCP connection running; all survive."""
    cdn = t.service("cdn")
    manifest = t.manifest()
    exe = t.install()
    server = t.server(exe=exe, state=t.root / "state")
    t.eq(server.version(), manifest["version"], "the installed version before")
    d = t.driver()
    hello = d.connect(server)
    t.eq(hello["server_version"], manifest["version"], "the server's version before")

    live = LivePanes(t, d, server, echo=t.service("echo"))
    t.eq(len(live.pane_pids), 8, "eight pane processes")
    before_lost = d.mark()

    started = time.monotonic()
    r = server.cmd("update", "--cdn", cdn, timeout=180)
    t.eq(r.code, 0, "tilrd update exits 0")
    t.has(r.out, manifest["next_version"])
    t.eventually(lambda: server.logged(RELOADED), "the server reloads", timeout=30)
    t.metric("update_to_reload_ms", round((time.monotonic() - started) * 1000))
    d.wait_event("disconnected", since=before_lost, timeout=30)
    back = d.cmd("reconnect", timeout_ms=30_000)
    t.metric("reconnect_ms", back["ms"])
    t.metric("reload_ms", (reload_times(server) or [0])[-1])
    live.wait_settled()

    t.check("the executable is the next version", lambda: t.eq(server.version(), manifest["next_version"], "version"))
    t.check("the tilr next to it is the next version",
            lambda: t.has(t.run(str(exe.with_name("tilr")), "--version", quiet=True).out, manifest["next_version"]))
    t.check("the server says it is the next version",
            lambda: t.eq(back["server_version"], manifest["next_version"], "HelloAck.server_version"))
    t.check("the server keeps its pid and its listening socket",
            lambda: t.eq(t.listener_pid(server.port), server.pid, "listener pid"))
    t.check("it reloaded once, with all eight panes", lambda: t.eq(server.logged(f"{RELOADED} 8 pane"), 1, "reloads"))
    live.check("the update")

    def descriptors_kept():
        # Exactly as many: a process holds the same descriptors whichever executable
        # runs in it (each child is watched through a pidfd, spawned or inherited).
        now = live.descriptors()
        t.metric("descriptors_before_after", {pid: [live.fds.get(pid), n] for pid, n in now.items()})
        changed = {pid: (live.fds.get(pid), n) for pid, n in now.items() if n != live.fds.get(pid)}
        t.eq(changed, {}, "processes holding another number of descriptors than before the update "
                          "(pid: before, after)")
    t.check("no descriptor leaks in the supervisor or the pane processes", descriptors_kept)

    live.finish()

    def second_update_does_nothing():
        again = server.cmd("update", "--cdn", cdn, timeout=60)
        t.eq(again.code, 0, "exit status")
        t.has(again.out, "is the latest release")
        time.sleep(3)
        t.eq(server.logged(RELOADED), 1, "reloads")
    t.check("a second update finds nothing to do", second_update_does_nothing)

    t.check("nothing was refused or lost in the log",
            lambda: t.eq(re.findall(r"not reloading|ERROR|panicked", server.log_text()), [], "bad log lines"))


@case(fixture="server_state", services=["cdn"], timeout=240, spec=[
    "update/the-installed-release-is-on-disk-before-success#synced-before-success",
])
def an_update_is_synced_before_it_reports_success(t: T):
    """UC-060: under strace, `tilrd update` fsyncs both unpacked programs before the first rename and the directory after the second, before `updated`."""
    cdn = t.service("cdn")
    manifest = t.manifest()
    exe = t.install()
    dir = os.path.realpath(exe.parent)
    trace = t.scratch("strace") / "update.trace"
    r = t.run(*strace(trace, calls="fsync,fdatasync,rename,renameat,renameat2,write"), exe, "update", "--cdn", cdn,
              env=t.server_env(t.scratch("updating")), timeout=180)
    t.eq(r.code, 0, "tilrd update exits 0")
    t.has(r.out, manifest["next_version"])
    calls = syscalls(trace)
    staged = re.escape(f"{dir}/.tilr-update.")
    print("\n".join(f"{name}({args[:160]}" for name, args in calls
                    if name != "write" or args.startswith("1<")))

    def first(what: str, match) -> int:
        found = [i for i, (name, args) in enumerate(calls) if match(name, args)]
        t.ok(found, f"{what} is in the trace")
        return found[0]
    renames = [i for i, (name, args) in enumerate(calls) if name.startswith("rename") and re.search(staged, args)
               and not args.rstrip().endswith("= -1")]
    t.ok(len(renames) >= 2, f"two renames out of the staging directory ({len(renames)})")
    t.has(calls[renames[0]][1], f'"{dir}/tilr"')  # the first rename puts tilr in place
    t.has(calls[renames[-1]][1], f'"{dir}/{exe.name}"')  # the server last
    for program in ("tilr", "tilrd"):
        synced = first(f"an fsync of the unpacked {program}", lambda name, args: name in ("fsync", "fdatasync")
                       and re.match(rf"\d+<{staged}\d+/{program}>\)", args))
        t.ok(synced < renames[0], f"{program} is synced ({synced}) before the first rename ({renames[0]})")
    dir_synced = first("an fsync of the executable's directory after the renames", lambda name, args: name == "fsync"
                       and re.match(rf"\d+<{re.escape(dir)}>\)", args))
    printed = first("`updated` on stdout", lambda name, args: name == "write" and args.startswith("1<")
                    and '"updated ' in args)
    t.ok(renames[-1] < dir_synced < printed,
         f"last rename ({renames[-1]}) < fsync of the directory ({dir_synced}) < `updated` ({printed})")
