"""UC-071: what a file is besides its bytes survives the files protocol: names with spaces,
quotes, control characters, Unicode (Cyrillic, emoji, both normal forms of "é"), names
that are not UTF-8 at all, symbolic links (to a file, to a directory, broken, leading out
of the shared directories) and permission bits.

Names: a file written under a name has exactly that name on disk (bytes compared), is
listed under it, and reads back. A name that is not UTF-8 (Linux allows any bytes but
'/' and NUL) is made on the server's side: it must be listed in a way that names it, so
that a client can read it back. tilr.proto carries paths as `string` (UTF-8 only), so
such a name has no exact spelling on the wire; the case asks that the listing still
leads to the file.
Links: listed as links with what they point to (FsEntry.target), read and written
through (the link stays a link), refused when they lead out of the shared directories.
Modes: a write over a file keeps its mode (0750, 0600, setgid), a new file gets 0666
less the umask, FsEntry.mode says the same as stat.

Was red until 2026-10-17: a name that is not UTF-8 was listed lossily and the listed name
led to no file. Now tilr.proto carries such names as raw bytes beside the string
(FsEntry.name_bytes, `path_bytes` and the like in every request and answer), and
tilr-driver writes them in its JSON as Python's surrogateescape (`\\udcXX`), so the name the
case gets from the listing is the file's exact bytes when it is sent back.
"""

import os
import stat
import unicodedata
from pathlib import Path

from tt import T, case

NAMES = [
    "with space.txt",
    "  leading and trailing  ",
    "quote's \"double\" `back`.txt",
    "tab\there.txt",
    "new\nline.txt",
    "-starts-with-dash",
    "юникод файл.txt",
    "emoji 🎉🚀.txt",
    unicodedata.normalize("NFC", "café-nfc.txt"),
    unicodedata.normalize("NFD", "café-nfd.txt"),
    "日本語のファイル名.md",
    "x" * 200 + ".long",
    "$HOME;`id`|&>.txt",
]


def start(t: T):
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server, features=["ping", "files"])
    home = Path(server.env["HOME"])
    work = home / "names"
    work.mkdir()
    return server, d, home, work


@case(fixture="server_state")
def odd_and_unicode_names_go_both_ways_and_keep_their_exact_bytes(t: T):
    """UC-071: spaces, quotes, tab, newline, Cyrillic, emoji, NFC/NFD, 200 chars: written, listed and read back exactly."""
    server, d, home, work = start(t)
    for n, name in enumerate(NAMES):
        content = f"content {n} of {name!r}\n"

        def one(name=name, content=content):
            ack = d.cmd("fs_write", path=str(work / name), text=content)
            t.eq(ack["error"], "", "written")
            on_disk = os.listdir(os.fsencode(work))
            t.ok(os.fsencode(name) in on_disk, f"the name on disk is exactly {name!r} ({on_disk[-3:]})")
            t.eq((work / name).read_text(), content, "the content on disk")
            back = d.cmd("fs_read", path=str(work / name))
            t.eq((back["error"], back["text"]), ("", content), "read back")
        t.check(f"name {name!r}", one)
    listed = d.cmd("fs_list", path=str(work))
    t.eq(listed["error"], "", "the directory is listed")
    t.eq(sorted(e["name"] for e in listed["entries"]), sorted(NAMES), "the listing has every name exactly")
    t.ok(unicodedata.normalize("NFD", "café-nfd.txt") in os.listdir(work)
         and unicodedata.normalize("NFC", "café-nfc.txt") in os.listdir(work),
         "both normal forms are kept as written, not normalized")


@case(fixture="server_state")
def a_name_that_is_not_utf8_is_listed_so_that_it_can_be_read_back(t: T):
    """UC-071: a file named with bytes that are not UTF-8: listed, and the listed name reads the file back."""
    server, d, home, work = start(t)
    raw = os.fsencode(work) + b"/bin\xff\xfe-\xc3(name.dat"
    with open(raw, "wb") as f:
        f.write(b"binary-named content\n")
    listed = d.cmd("fs_list", path=str(work))
    t.eq(listed["error"], "", "the directory is listed")
    t.eq(len(listed["entries"]), 1, f"the file is listed ({listed['entries']})")
    name = listed["entries"][0]["name"]
    print(f"  listed as {name!r}")
    back = d.cmd("fs_read", path=str(work / name))
    t.eq((back["error"], back["text"]), ("", "binary-named content\n"), "the listed name reads the file back")
    info = d.cmd("fs_stat", path=str(work / name))
    t.eq(info["error"], "", "and stats it")


@case(fixture="server_state")
def symbolic_links_are_listed_followed_and_written_through_but_not_out_of_the_shared_directories(t: T):
    """UC-071: links to a file, a directory, nothing and outside: kinds, reads, writes through, the outside one refused."""
    server, d, home, work = start(t)
    (work / "real.txt").write_text("real\n")
    (work / "dir").mkdir()
    (work / "dir" / "inner.txt").write_text("inner\n")
    os.symlink("real.txt", work / "to-file")
    os.symlink("dir", work / "to-dir")
    os.symlink("nowhere", work / "broken")
    outside = t.scratch("outside") / "secret.txt"
    outside.write_text("not shared\n")
    os.symlink(outside, work / "to-outside")

    entries = {e["name"]: e for e in d.cmd("fs_list", path=str(work))["entries"]}
    t.check("a link to a file is a link to a file",
            lambda: t.eq((entries["to-file"]["kind"], entries["to-file"]["target"]), ("link", "file"), "kind, target"))
    t.check("a link to a directory is a link to a directory",
            lambda: t.eq((entries["to-dir"]["kind"], entries["to-dir"]["target"]), ("link", "dir"), "kind, target"))
    t.check("a broken link is a link to nothing",
            lambda: t.eq((entries["broken"]["kind"], entries["broken"]["target"]), ("link", "other"), "kind, target"))
    t.check("a read through a link reads the file",
            lambda: t.eq(d.cmd("fs_read", path=str(work / "to-file"))["text"], "real\n", "content"))
    t.check("a directory is listed through a link",
            lambda: t.eq([e["name"] for e in d.cmd("fs_list", path=str(work / "to-dir"))["entries"]], ["inner.txt"],
                         "entries"))

    def written_through():
        t.eq(d.cmd("fs_write", path=str(work / "to-file"), text="new\n")["error"], "", "write")
        t.ok((work / "to-file").is_symlink(), "the link is still a link")
        t.eq(os.readlink(work / "to-file"), "real.txt", "pointing where it did")
        t.eq((work / "real.txt").read_text(), "new\n", "the file it points to has the new content")
    t.check("a write through a link replaces the file it points to", written_through)

    def outside_refused():
        read = d.cmd("fs_read", path=str(work / "to-outside"))
        t.ok(read["error"], f"reading through a link out of the shared directories is refused ({read['error']!r})")
        write = d.cmd("fs_write", path=str(work / "to-outside"), text="overwritten\n")
        t.ok(write["error"], f"and writing ({write['error']!r})")
        t.eq(outside.read_text(), "not shared\n", "the file outside is untouched")
    t.check("a link out of the shared directories leads nowhere", outside_refused)


@case(fixture="server_state")
def permission_bits_are_kept_by_writes_and_told_by_listings(t: T):
    """UC-071: writes over 0750, 0600 and 2775 files keep the mode; a new file is 0666 less the umask; modes listed."""
    server, d, home, work = start(t)
    modes = {"script.sh": 0o750, "private.key": 0o600, "shared.dir-file": 0o2775}
    for name, mode in modes.items():
        (work / name).write_text("old\n")
        os.chmod(work / name, mode)
    for name, mode in modes.items():
        def kept(name=name, mode=mode):
            t.eq(d.cmd("fs_write", path=str(work / name), text="new\n")["error"], "", "written")
            t.eq((work / name).read_text(), "new\n", "content")
            t.eq(oct(stat.S_IMODE((work / name).stat().st_mode)), oct(mode), "mode on disk")
        t.check(f"a write over a {oct(mode)} file keeps its mode", kept)
    umask = os.umask(0)
    os.umask(umask)
    t.eq(d.cmd("fs_write", path=str(work / "fresh.txt"), text="x")["error"], "", "a new file")
    t.check("a new file is 0666 less the server's umask",
            lambda: t.eq(oct(stat.S_IMODE((work / "fresh.txt").stat().st_mode)), oct(0o666 & ~umask), "mode"))
    listed = {e["name"]: e["mode"] for e in d.cmd("fs_list", path=str(work))["entries"]}
    t.check("the listing says the mode stat says",
            lambda: t.eq({n: oct(m) for n, m in listed.items()},
                         {n: oct(stat.S_IMODE((work / n).stat().st_mode)) for n in listed}, "modes"))
