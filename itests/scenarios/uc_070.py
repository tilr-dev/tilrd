"""UC-070: files both ways (FsWrite up, FsRead down) at the edges of size: 0 bytes, 1 byte,
exactly the server's limit, the limit plus one, and a gigabyte in chunks; what arrives
hashes to what was sent.

The limit is `[files] max_bytes` of the server's configuration file (HelloAck.fs_max_bytes):
1 MiB for the edge cases, 1 GiB for the gigabyte (which is then exactly the limit). One
FsWrite carries a whole file; more than the limit is refused before anything is written
(no file, no temporary file). One FsRead returns at most the limit: a larger file is read
in parts (offset, length), as tilr.proto says. Contents are random, so compression does
not make the transfer look faster than it is. The client's files are on the case's side
of the driver (/tmp/tt), the server's in its home directory.
"""

import hashlib
import os
from pathlib import Path

from tt import T, case

LIMIT = 1 << 20
GIB = 1 << 30


def configured(t: T, max_bytes: str):
    config = t.scratch("config")
    (config / "server.toml").write_text(f'[files]\nmax_bytes = "{max_bytes}"\n')
    server = t.server(state=t.root / "state", env={"TILR_CONFIG_DIR": str(config)})
    d = t.driver()
    hello = d.connect(server, features=["files"])
    return server, d, hello, Path(server.env["HOME"])


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def random_file(path: Path, size: int) -> Path:
    with open(path, "wb") as f:
        left = size
        while left:
            n = min(left, 1 << 20)
            f.write(os.urandom(n))
            left -= n
    return path


@case(fixture="server_state", spec=["files/reads-come-in-chunks", "files/writes-are-whole-or-not-at-all"])
def files_of_zero_one_and_exactly_the_limit_go_both_ways_intact(t: T):
    """UC-070: 0 B, 1 B and exactly the limit (1 MiB): up and down, hashes equal, sizes and stat right."""
    server, d, hello, home = configured(t, "1MB")
    t.eq(hello["fs_max_bytes"], LIMIT, "HelloAck.fs_max_bytes is the configured limit")
    local = t.scratch("client")
    for size in (0, 1, LIMIT):
        name = f"f{size}.bin"
        src = random_file(local / name, size)
        up = d.cmd("fs_write", path=f"~/{name}", **{"from": str(src)}, chunk=64 * 1024)
        t.eq(up["error"], "", f"{size} B written")
        t.eq(sha(home / name), sha(src), f"{size} B: the server's file has what was sent")
        info = d.cmd("fs_stat", path=f"~/{name}")
        t.eq((info["error"], info["entry"]["size"], info["entry"]["kind"]), ("", size, "file"), f"{size} B: stat")
        back = local / f"back-{name}"
        down = d.cmd("fs_read", path=f"~/{name}", to=str(back))
        t.eq((down["error"], down["size"], down["file_size"]), ("", size, size), f"{size} B read")
        t.eq(sha(back), sha(src), f"{size} B: what came back is what was sent")
        t.eq(down["sha256"], sha(src), f"{size} B: the driver's hash of what it read")
    t.eq(sorted(p.name for p in home.glob(".*tilr-write-*")), [], "no temporary files")


@case(fixture="server_state", spec=["files/a-read-is-bounded-by-the-limit-and-fails-whole"])
def a_file_one_byte_over_the_limit_is_refused_whole_and_read_in_parts(t: T):
    """UC-070: limit+1: the write is refused with nothing written; one read refused, reading in parts works."""
    server, d, hello, home = configured(t, "1MB")
    local = t.scratch("client")
    src = random_file(local / "over.bin", LIMIT + 1)
    up = d.cmd("fs_write", path="~/over.bin", **{"from": str(src)})
    t.ok(up["error"], f"the write is refused: {up['error']!r}")
    t.ok("too large" in up["error"], "and says it is too large")
    # The refusal comes as soon as the first chunk is read; what the client sent until it
    # heard it is ignored (tilr.proto, FsWrite).
    t.metric("sent_before_refusal", up["sent"])
    t.ok(not (home / "over.bin").exists(), "no file was made")
    t.eq(sorted(p.name for p in home.glob(".*tilr-write-*")), [], "no temporary file")

    # The same content made on the server's side, then read.
    (home / "over.bin").write_bytes(src.read_bytes())
    whole = d.cmd("fs_read", path="~/over.bin", to=str(local / "whole"))
    t.ok(whole["error"] and "too large" in whole["error"], f"one read of limit+1 is refused: {whole['error']!r}")
    parts = d.cmd("fs_read", path="~/over.bin", to=str(local / "parts"), part=LIMIT)
    t.eq((parts["error"], parts["size"], parts["parts"]), ("", LIMIT + 1, 2), "read in two parts of at most the limit")
    t.eq(sha(local / "parts"), sha(src), "the parts make the file")
    tail = d.cmd("fs_read", path="~/over.bin", offset=LIMIT, length=1)
    t.eq((tail["error"], tail["size"]), ("", 1), "a read of the last byte")


@case(fixture="server_state", timeout=900, spec=["files/file-content-goes-by-the-bulk-lane"])
def a_gigabyte_goes_up_and_comes_back_in_chunks_with_its_hash(t: T):
    """UC-070: 1 GiB up in 1 MiB chunks, down in 64 MiB parts; hashes equal; throughput and server memory recorded."""
    server, d, hello, home = configured(t, "1GB")
    t.eq(hello["fs_max_bytes"], GIB, "the limit is a gigabyte")
    local = t.scratch("client")
    src = random_file(local / "big.bin", GIB)
    want = sha(src)
    rss0 = t.proc(server.pid).rss_kb
    up = d.cmd("fs_write", path="~/big.bin", **{"from": str(src)}, chunk=1 << 20, timeout_ms=600_000)
    t.eq(up["error"], "", "the gigabyte is written")
    t.metric("up_mb_s", round(GIB / 1e6 / (up["ms"] / 1000), 1))
    t.metric("server_rss_growth_kb", t.proc(server.pid).rss_kb - rss0)
    t.eq(sha(home / "big.bin"), want, "the server's file hashes to what was sent")
    back = local / "back.bin"
    down = d.cmd("fs_read", path="~/big.bin", to=str(back), part=64 << 20, timeout_ms=600_000)
    t.eq((down["error"], down["size"], down["parts"]), ("", GIB, 16), "read back in 16 parts")
    t.metric("down_mb_s", round(GIB / 1e6 / (down["ms"] / 1000), 1))
    t.eq(sha(back), want, "what came back hashes to what was sent")
    t.eq(sorted(p.name for p in home.glob(".*tilr-write-*")), [], "no temporary file")
    peak = int(next((l.split()[1] for l in Path(f"/proc/{server.pid}/status").read_text().splitlines()
                     if l.startswith("VmHWM:")), 0))
    t.metric("server_peak_rss_kb", peak)
    t.ok(peak < 512 * 1024, f"the server never held the file in memory (peak RSS {peak} kB)")
