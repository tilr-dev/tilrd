"""UC-072: several files go both ways at once on the client's connection while the user
types in a pane: the terminal does not slow down, and every file arrives intact.

Also each direction alone, to tell where a slowdown comes from: frames queued by the
server behind file data going down, or keystrokes behind the client's own chunks going
up. One connection: four writes of 32 MiB (FsWrite, 256 KiB chunks) and two reads of 32 MiB
(FsRead in 8 MiB parts) run in the background of the driver, which keeps serving the
pane meanwhile; the case types a character into an idle shell every 50 ms and the driver
measures each one's latency (input sent -> the first frame carrying its seq). The same
typing without transfers is the baseline. Recorded: p50/p90/p99/max of both, the
transfers' throughput. Asserted: generous bounds (no tuned thresholds yet, see group H):
p99 under transfers within 250 ms or 5x the baseline (whichever is more), no keystroke
over a second, every keystroke echoed, every hash equal.

Red (2026-10-17): reads stall the terminal. Three 32 MiB reads (8 MiB parts) put a
keystroke's echo up to 1.6 s behind (p50 0.6 s; baseline 1 ms); with writes too, up to
5 s. A part is answered as fast as the disk gives it (crates/tilr-server/src/files.rs:458,
256 KiB FsData in a loop) into the client's one FIFO queue (outbox.rs), where the frames
wait behind it: nothing puts the screen ahead of file data. Writes alone are fine (p99
53 ms).
"""

import hashlib
import os
import time
from pathlib import Path

from tt import T, case

SIZE = 32 << 20
WRITES, READS = 4, 2


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def typing(d, pane, seconds: float, until=None) -> dict:
    """Type a character every 50 ms for `seconds` (or until `until()` is true); the
    latency the driver measured."""
    d.latency(reset=True)
    end = time.monotonic() + seconds
    n = 0
    while time.monotonic() < end and not (until and until()):
        d.cmd("input", pane=pane, text="x", quiet=True)
        n += 1
        time.sleep(0.05)
    d.cmd("sleep", ms=500)
    d.input(pane, "\x15")  # Ctrl-U: the typed line goes
    return d.latency()


def transfers_while_typing(t: T, writes: int, reads: int):
    server = t.server(state=t.root / "state")
    home = Path(server.env["HOME"])
    d = t.driver()
    d.connect(server, features=["ping", "files"])
    pane = d.shell_pane()
    local = t.scratch("client")
    ups = []
    for n in range(writes):
        src = local / f"up{n}.bin"
        src.write_bytes(os.urandom(SIZE))
        ups.append(src)
    downs = []
    for n in range(reads):
        src = home / f"down{n}.bin"
        src.write_bytes(os.urandom(SIZE))
        downs.append(src)

    base = typing(d, pane, 5)
    t.metric("baseline_latency_ms", {k: base[k] for k in ("count", "p50_ms", "p90_ms", "p99_ms", "max_ms")})

    started = time.monotonic()
    transfers = []
    for n, src in enumerate(ups):
        r = d.cmd("fs_write", path=f"~/up{n}.bin", **{"from": str(src)}, chunk=256 * 1024, background=True)
        transfers.append(("up", n, r["transfer"]))
    for n, src in enumerate(downs):
        r = d.cmd("fs_read", path=str(src), to=str(local / f"down{n}.bin"), part=8 << 20, background=True)
        transfers.append(("down", n, r["transfer"]))
    ids = {tr[2] for tr in transfers}
    done = lambda: ids <= {e.get("transfer") for e in d.events if e.get("ev") == "fs_done"}
    busy = typing(d, pane, 300, until=done)
    t.metric("latency_during_transfers_ms", {k: busy[k] for k in ("count", "p50_ms", "p90_ms", "p99_ms", "max_ms")})
    outcomes = {tr: d.cmd("fs_wait", transfer=tr[2], timeout_ms=300_000) for tr in transfers}
    seconds = time.monotonic() - started
    t.metric("transfers_s", round(seconds, 1))
    t.metric("transfers_mb_s", round((writes + reads) * SIZE / 1e6 / seconds, 1))

    for (way, n, _), out in outcomes.items():
        t.check(f"{way} {n} succeeded", lambda out=out: t.eq(out["error"], "", "error"))
    for n, src in enumerate(ups):
        t.check(f"up {n} arrived intact", lambda n=n, src=src: t.eq(sha(home / f"up{n}.bin"), sha(src), "sha256"))
    for n, src in enumerate(downs):
        t.check(f"down {n} arrived intact",
                lambda n=n, src=src: t.eq(sha(local / f"down{n}.bin"), sha(src), "sha256"))
    t.eq(busy["unanswered"], 0, "every keystroke was echoed")
    t.ok(busy["max_ms"] <= 1000, f"no keystroke waited a second ({busy['max_ms']} ms)")
    t.ok(busy["p99_ms"] <= max(250, 5 * base["p99_ms"]),
         f"p99 latency during the transfers {busy['p99_ms']} ms stays within 250 ms or 5x the baseline "
         f"({base['p99_ms']} ms)")


@case(fixture="server_state", timeout=600)
def parallel_transfers_both_ways_leave_the_terminal_responsive_and_the_files_intact(t: T):
    """UC-072: 4 x 32 MiB up and 2 x 32 MiB down at once while typing every 50 ms: latency bounded, hashes equal."""
    transfers_while_typing(t, WRITES, READS)


@case(fixture="server_state", timeout=600)
def parallel_reads_leave_the_terminal_responsive(t: T):
    """UC-072: 3 x 32 MiB down at once while typing (what the server queues ahead of the frames)."""
    transfers_while_typing(t, 0, 3)


@case(fixture="server_state", timeout=600)
def parallel_writes_leave_the_terminal_responsive(t: T):
    """UC-072: 3 x 32 MiB up at once while typing (what the client's own uploads put ahead of its keys)."""
    transfers_while_typing(t, 3, 0)
