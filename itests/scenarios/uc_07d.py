"""UC-07D: copying from a pane to the client: what a program in a pane copies (vim writing
to `xclip -selection clipboard -i`, OSC 52 in its output, `xclip -t image/png -i`) reaches
the client that watches the pane and lists "clipboard-write" in Hello.features, as
ClipboardWrite, byte for byte; a client watching it without "clipboard-write" gets nothing.

`xclip` is the server's on the panes' PATH ($TILR_SHIMS); the image is random bytes behind
a PNG signature, so nothing about it is text.
"""

import hashlib
import os

from tt import T, case


def copies(d, since: int) -> list[dict]:
    return [e for e in d.events[since:] if e.get("ev") in ("clipboard_write", "clipboard_write_cut")]


@case(fixture="server_state", spec=["clipboard-host-events/a-copy-in-a-pane-reaches-the-clients"])
def a_copy_in_vim_reaches_the_clients_clipboard(t: T):
    """UC-07D: vim's :w !xclip, OSC 52 and an xclip image reach the copying client whole, not the other."""
    server = t.server(state=t.root / "state")
    desk = t.driver("desk")
    desk.connect(server, features=["host-events", "clipboard-write"])
    pane = desk.shell_pane()
    viewer = t.driver("viewer")
    viewer.connect(server)
    viewer.cmd("subscribe", pane=pane)
    t.eventually(lambda: desk.viewers(pane) == 2, "two clients watch the pane", timeout=10)
    seen_by_viewer = viewer.mark()

    notes = t.scratch("vim") / "notes.txt"
    notes.write_text("copied from vim\n")
    mark = desk.mark()
    desk.line(pane, f"vim -n -u NONE -N {notes}")
    desk.wait_for(pane, r"(?m)^copied from vim", timeout=15)
    desk.line(pane, ":w !xclip -selection clipboard -i")
    copy = desk.wait_event("clipboard_write", since=mark, timeout=15)
    t.eq((copy["pane"], copy["mime"], copy["text"]), (pane, "text/plain;charset=utf-8", "copied from vim\n"),
         "vim's copy reaches the client")
    desk.key(pane, "enter")
    desk.line(pane, ":q!")
    desk.wait_for(pane, r"(?m)^\$ ?$", timeout=10)

    mark = desk.mark()
    t.eq(desk.run(pane, r"printf '\033]52;c;%s\007' $(printf 'from OSC 52' | base64)"), 0, "printf of OSC 52")
    copy = desk.wait_event("clipboard_write", since=mark, timeout=15)
    t.eq((copy["pane"], copy["text"]), (pane, "from OSC 52"), "an OSC 52 copy reaches the client")

    image = b"\x89PNG\r\n\x1a\n" + os.urandom(700 * 1024)
    src = t.scratch("image") / "shot.png"
    src.write_bytes(image)
    mark = desk.mark()
    t.eq(desk.run(pane, f"xclip -selection clipboard -t image/png -i < {src}"), 0, "xclip copies the image")
    copy = desk.wait_event("clipboard_write", since=mark, timeout=15)
    t.eq((copy["mime"], copy["size"], copy["chunks"]), ("image/png", len(image), 3),
         "the image comes in 256 KiB chunks")
    t.eq(copy["sha256"], hashlib.sha256(image).hexdigest(), "byte for byte")
    t.eq(copies(desk, mark)[:-1], [], "no copy was cut short")
    t.eq(copies(viewer, seen_by_viewer), [], "the client without \"clipboard-write\" got nothing")
