"""UC-075: the shared clipboard: content a client uploads (ClipboardPut in chunks) is what
programs in the panes paste, byte for byte, through the `xclip` and `wl-paste` the server
puts on their PATH; the size limit holds exactly; content expires.

The server runs with --clipboard-max 1MB (HelloAck.clipboard_max_bytes) and
--clipboard-ttl 3. The image is a real PNG (made here, with a CRC per chunk) with random
pixels, so nothing about it is text.
"""

import hashlib
import os
import struct
import time
import zlib

from tt import T, case

LIMIT = 1 << 20
TTL_S = 3


def png(width: int, height: int) -> bytes:
    """A valid PNG of random RGB pixels."""
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + os.urandom(width * 3) for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 0)) + chunk(b"IEND", b""))


def start(t: T):
    server = t.server("--clipboard-max", "1MB", "--clipboard-ttl", TTL_S, state=t.root / "state")
    d = t.driver()
    hello = d.connect(server)
    pane = d.shell_pane()
    return server, d, hello, pane


def pasted(t: T, d, pane, command: str) -> tuple[int, str]:
    """Run a paste command in the pane into a file; its status and the file's sha256."""
    out = t.scratch("paste") / "out"
    code = d.run(pane, f"{command} >{out}")
    return code, hashlib.sha256(out.read_bytes()).hexdigest() if out.exists() else ""


@case(fixture="server_state", spec=["clipboard-host-events/an-upload-becomes-the-clipboard-on-its-last-chunk"])
def an_image_put_on_the_clipboard_is_pasted_byte_for_byte(t: T):
    """UC-075: a 700 KB PNG in 64 KiB chunks: xclip and wl-paste in a pane give it back exactly; text too."""
    server, d, hello, pane = start(t)
    t.eq(hello["features"].count("clipboard"), 1, "the server offers the clipboard")
    image = png(400, 580)
    src = t.scratch("image") / "shot.png"
    src.write_bytes(image)
    ack = d.cmd("clipboard_put", mime="image/png", **{"from": str(src)}, chunk=64 * 1024)
    t.eq((ack["ok"], ack["error"]), (True, ""), f"the upload of {len(image)} B is taken")
    want = hashlib.sha256(image).hexdigest()
    t.has(d.value(pane, "$(xclip -selection clipboard -t TARGETS -o | tr '\\n' ' ')"), "image/png")
    t.eq(pasted(t, d, pane, "xclip -selection clipboard -t image/png -o"), (0, want), "xclip pastes the image")
    t.eq(pasted(t, d, pane, "wl-paste --type image/png"), (0, want), "wl-paste pastes the image")
    text = "строка 1\n\ttab and emoji 🎉\nlast line without newline"
    ack = d.cmd("clipboard_put", mime="text/plain;charset=utf-8", text=text)
    t.ok(ack["ok"], "the text is taken")
    t.eq(pasted(t, d, pane, "xclip -selection clipboard -o"), (0, hashlib.sha256(text.encode()).hexdigest()),
         "xclip pastes the text exactly")


@case(fixture="server_state", spec=["clipboard-host-events/the-first-chunk-empties-the-clipboard"])
def the_clipboard_takes_exactly_its_limit_and_refuses_one_byte_more_leaving_it_empty(t: T):
    """UC-075: exactly 1 MiB is taken and pasted; 1 MiB + 1 is refused and the clipboard is empty; a lying size refused."""
    server, d, hello, pane = start(t)
    t.eq(hello["clipboard_max_bytes"], LIMIT, "HelloAck.clipboard_max_bytes is the limit")
    exact = os.urandom(LIMIT)
    src = t.scratch("clip") / "exact.bin"
    src.write_bytes(exact)
    ack = d.cmd("clipboard_put", mime="application/octet-stream", **{"from": str(src)})
    t.eq((ack["ok"], ack["error"]), (True, ""), "exactly the limit is taken")
    t.eq(pasted(t, d, pane, "xclip -selection clipboard -t application/octet-stream -o"),
         (0, hashlib.sha256(exact).hexdigest()), "and pasted whole")

    over = t.scratch("clip") / "over.bin"
    over.write_bytes(os.urandom(LIMIT + 1))
    ack = d.cmd("clipboard_put", mime="application/octet-stream", **{"from": str(over)})
    t.eq(ack["ok"], False, f"one byte more is refused ({ack['error']})")
    t.ok("too large" in ack["error"], "and the refusal says why")
    code, digest = pasted(t, d, pane, "xclip -selection clipboard -t application/octet-stream -o")
    t.ok(code != 0 or digest == hashlib.sha256(b"").hexdigest(),
         f"the clipboard is empty after the refusal (status {code})")

    ack = d.cmd("clipboard_put", mime="text/plain", text="twenty bytes of text", total_size=10)
    t.eq(ack["ok"], False, f"a transfer carrying more than it announced is refused ({ack['error']})")


@case(fixture="server_state", spec=["clipboard-host-events/content-expires"])
def clipboard_content_expires(t: T):
    """UC-075: with --clipboard-ttl 3, content pastes at once and is gone after the TTL."""
    server, d, hello, pane = start(t)
    ack = d.cmd("clipboard_put", mime="text/plain", text="short-lived")
    t.ok(ack["ok"], "taken")
    put = time.monotonic()
    t.eq(d.value(pane, "$(xclip -selection clipboard -o)"), "short-lived", "pasted at once")
    time.sleep(max(0, TTL_S + 1 - (time.monotonic() - put)))
    code, digest = pasted(t, d, pane, "xclip -selection clipboard -o")
    t.ok(code != 0 or digest == hashlib.sha256(b"").hexdigest(),
         f"after {TTL_S} s the clipboard has nothing to paste (status {code})")
