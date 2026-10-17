"""UC-079: input as a program in the pane receives it: Ctrl codes, Alt, function and
cursor keys (KeyInput), bracketed paste (Paste), the mouse in an application that asked
for it (MouseInput), and Ctrl+C typed on a Russian layout, which must be ^C.

The program (KEYLOG below) puts its terminal in raw mode, turns on what it is told
(bracketed paste, SGR mouse reporting), and appends every byte it reads, as hex, to a
log the case reads: what the pty delivered, nothing interpreted. The expected bytes are
what xterm sends (the TERM of the panes is xterm-256color): normal cursor-key mode,
CSI ~ for the editing and F5-F12 keys, SS3 for F1-F4.

Ctrl+C on a Russian layout: the key under C is "с" (U+0441) there. A client sends what
the layout makes of the key and which physical key it is, as a browser's KeyboardEvent
has them (key and code): KeyInput {codepoint 'с', text 'с', code "KeyC", mods ctrl}
(tilr-driver's `layout`). A terminal sends ^C for it, as for the same physical key on a
Latin layout: Ctrl or Alt with a key whose character is not ASCII goes by the physical
key's US character (Ctrl+С ^C, Ctrl+Я ^Z, Alt+Ф ESC a). A character that is ASCII goes
by itself, wherever its key is: Ctrl+C on Dvorak (the I key) is ^C, Ctrl+A on AZERTY
(the Q key) is ^A; plain Cyrillic typing is the text.
"""

import time

from tt import T, case

KEYLOG = r"""
import os, sys, termios, tty
log = open(sys.argv[1], "a", buffering=1)
modes = sys.argv[2:]
fd = sys.stdin.fileno()
tty.setraw(fd)
if "paste" in modes:
    sys.stdout.write("\x1b[?2004h")
if "mouse" in modes:
    sys.stdout.write("\x1b[?1000h\x1b[?1002h\x1b[?1006h")
sys.stdout.write("KEYLOG READY\r\n")
sys.stdout.flush()
while True:
    data = os.read(fd, 4096)
    if not data:
        break
    log.write(data.hex() + "\n")
"""

KEYS = {
    "ctrl-a": b"\x01", "ctrl-b": b"\x02", "ctrl-c": b"\x03", "ctrl-d": b"\x04", "ctrl-e": b"\x05",
    "ctrl-h": b"\x08", "ctrl-k": b"\x0b", "ctrl-l": b"\x0c", "ctrl-q": b"\x11", "ctrl-s": b"\x13",
    "ctrl-u": b"\x15", "ctrl-w": b"\x17", "ctrl-z": b"\x1a",
    "alt-x": b"\x1bx", "alt-b": b"\x1bb", "alt-shift-x": b"\x1bX",
    "enter": b"\r", "tab": b"\t", "shift-tab": b"\x1b[Z", "backspace": b"\x7f", "esc": b"\x1b",
    "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D",
    "home": b"\x1b[H", "end": b"\x1b[F", "insert": b"\x1b[2~", "delete": b"\x1b[3~",
    "pageup": b"\x1b[5~", "pagedown": b"\x1b[6~",
    "f1": b"\x1bOP", "f2": b"\x1bOQ", "f3": b"\x1bOR", "f4": b"\x1bOS",
    "f5": b"\x1b[15~", "f6": b"\x1b[17~", "f7": b"\x1b[18~", "f8": b"\x1b[19~",
    "f9": b"\x1b[20~", "f10": b"\x1b[21~", "f11": b"\x1b[23~", "f12": b"\x1b[24~",
    "ctrl-up": b"\x1b[1;5A", "shift-f5": b"\x1b[15;2~",
}


class Keylog:
    def __init__(self, t: T, d, *modes: str):
        self.t, self.d = t, d
        self.pane = d.shell_pane()
        self.path = t.scratch("keylog") / "bytes"
        self.path.touch()
        program = t.root / "keylog.py"
        program.write_text(KEYLOG)
        d.line(self.pane, f"python3 {program} {self.path} {' '.join(modes)}")
        d.wait_for(self.pane, r"KEYLOG READY")

    def read(self) -> bytes:
        return b"".join(bytes.fromhex(l) for l in self.path.read_text().split())

    def after(self, send, settle: float = 0.3) -> bytes:
        """What the program read because of `send()`."""
        before = len(self.read())
        send()
        self.t.eventually(lambda: len(self.read()) > before, "the program reads something", timeout=5,
                          interval=0.02)
        time.sleep(settle)
        return self.read()[before:]


def start(t: T):
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    return server, d


@case(fixture="server_state")
def ctrl_alt_function_and_cursor_keys_reach_the_program_as_xterm_sends_them(t: T):
    """UC-079: Ctrl-A..Z, Alt, F1-F12, arrows, editing keys: the program reads xterm's bytes for each."""
    server, d = start(t)
    log = Keylog(t, d)
    wrong = {}
    for key, want in KEYS.items():
        got = log.after(lambda key=key: d.key(log.pane, key))
        if got != want:
            wrong[key] = f"{got!r} (expected {want!r})"
    t.metric("keys_checked", len(KEYS))
    t.eq(wrong, {}, "keys whose bytes differ from xterm's")


@case(fixture="server_state")
def bracketed_paste_wraps_the_text_and_cannot_be_ended_by_the_text(t: T):
    """UC-079: with ?2004 on a paste comes inside ESC[200~ ... ESC[201~; a pasted ESC[201~ cannot close it early."""
    server, d = start(t)
    log = Keylog(t, d, "paste")
    text = "line one\nline two\ttabbed\nрусский текст"
    got = log.after(lambda: d.cmd("paste", pane=log.pane, text=text))
    t.ok(got.startswith(b"\x1b[200~") and got.endswith(b"\x1b[201~"), f"the paste is bracketed: {got!r}")
    inner = got[len(b"\x1b[200~"):-len(b"\x1b[201~")]
    t.eq(inner.replace(b"\r\n", b"\n").replace(b"\r", b"\n"), text.encode(), "the text inside, newlines as CR or LF")
    evil = "harmless\x1b[201~rm -rf ~\n"
    got = log.after(lambda: d.cmd("paste", pane=log.pane, text=evil))
    t.eq(got.count(b"\x1b[201~"), 1, f"only the terminal's own end marker: {got!r}")
    t.ok(got.endswith(b"\x1b[201~"), "and it comes last")


@case(fixture="server_state")
def a_paste_without_bracketed_mode_is_plain_text(t: T):
    """UC-079: with ?2004 off a paste is the text alone, no markers."""
    server, d = start(t)
    log = Keylog(t, d)
    got = log.after(lambda: d.cmd("paste", pane=log.pane, text="plain paste"))
    t.eq(got, b"plain paste", "the text, nothing else")


@case(fixture="server_state")
def the_mouse_reaches_an_application_that_asked_for_it_and_no_other(t: T):
    """UC-079: SGR mouse on: press, release, drag and wheel at cells as ESC[<b;x;yM/m; off: nothing is sent."""
    server, d = start(t)
    plain = Keylog(t, d)
    d.cmd("mouse", pane=plain.pane, action="press", button="left", x=3, y=2)
    d.cmd("mouse", pane=plain.pane, action="release", button="left", x=3, y=2)
    time.sleep(0.5)
    t.eq(plain.read(), b"", "a program that did not ask for the mouse reads nothing")

    log = Keylog(t, d, "mouse")
    t.eventually(lambda: d.screen(log.pane)["mouse_tracking"], "Frame.mouse_tracking says the program wants the mouse")
    cases = [
        (dict(action="press", button="left", x=5, y=3), b"\x1b[<0;6;4M"),
        (dict(action="release", button="left", x=5, y=3), b"\x1b[<0;6;4m"),
        (dict(action="press", button="right", x=0, y=0), b"\x1b[<2;1;1M"),
        (dict(action="release", button="right", x=0, y=0), b"\x1b[<2;1;1m"),
        (dict(action="press", button="left", x=10, y=5), b"\x1b[<0;11;6M"),
        (dict(action="motion", button="left", x=12, y=5), b"\x1b[<32;13;6M"),
        (dict(action="release", button="left", x=12, y=5), b"\x1b[<0;13;6m"),
        (dict(action="press", button="wheel_up", x=7, y=7), b"\x1b[<64;8;8M"),
        (dict(action="press", button="wheel_down", x=7, y=7), b"\x1b[<65;8;8M"),
        (dict(action="press", button="left", x=1, y=1, mods=4), b"\x1b[<16;2;2M"),
    ]
    wrong = {}
    for args, want in cases:
        got = log.after(lambda args=args: d.cmd("mouse", pane=log.pane, **args), settle=0.15)
        if got != want:
            wrong[str(args)] = f"{got!r} (expected {want!r})"
    t.eq(wrong, {}, "mouse events whose bytes differ from xterm's SGR reports")


@case(fixture="server_state")
def ctrl_c_on_a_russian_layout_is_ctrl_c(t: T):
    """UC-079: Ctrl+С on a Russian layout (text 'с', code KeyC) reads as ^C and interrupts sleep; Dvorak, AZERTY too."""
    server, d = start(t)
    log = Keylog(t, d)
    cases = {
        ("ru", "ctrl-с"): b"\x03",  # the C key
        ("ru", "ctrl-я"): b"\x1a",  # the Z key
        ("ru", "ctrl-ц"): b"\x17",  # the W key
        ("ru", "alt-ф"): b"\x1ba",  # the A key
        ("ru", "с"): "с".encode(),  # typing is the text
        ("ru", "shift-я"): "Я".encode(),
        ("dvorak", "ctrl-c"): b"\x03",  # on the I key: the character wins
        ("dvorak", "ctrl-x"): b"\x18",  # on the B key
        ("azerty", "ctrl-a"): b"\x01",  # on the Q key
        ("azerty", "ctrl-w"): b"\x17",  # on the Z key
        ("us", "ctrl-c"): b"\x03",
    }
    wrong = {}
    for (layout, key), want in cases.items():
        got = log.after(lambda layout=layout, key=key: d.key(log.pane, key, layout=layout))
        if got != want:
            wrong[f"{layout} {key}"] = f"{got!r} (expected {want!r})"
    t.metric("keys_checked", len(cases))
    t.eq(wrong, {}, "keys on other layouts whose bytes differ from what the same key sends on a US layout")

    def interrupts_sleep():
        pane = d.shell_pane()
        d.line(pane, "sleep 1000")
        t.eventually(lambda: t.procs(r"^sleep 1000$"), "sleep runs")
        d.key(pane, "ctrl-с", layout="ru")
        t.eventually(lambda: not t.procs(r"^sleep 1000$"), "Ctrl+С on a Russian layout stops sleep", timeout=3)
    t.check("Ctrl+С interrupts a foreground program", interrupts_sleep)
