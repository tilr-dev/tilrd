"""UC-061: full-screen programs across a reload: vim (alternate screen, mouse on) and htop
(alternate screen, mouse on, cursor hidden) look the same afterwards, in the same terminal
modes, and still take keys and the mouse; vim's :wq writes the buffer it held.

The reload is the server's own (SIGUSR1: the supervisor and every pane process exec the
executable on disk). The screen model of each pane process is saved and restored across
its exec (PaneHandover.snapshot); what a client sees is its first full frame after it
reconnects: Frame.alt_screen, Frame.mouse_tracking, Frame.cursor.visible and the rows.
"""

import signal

from tt import T, case
from tt.live import reload_times, wait_reloads


def reload(t: T, server, d):
    since = d.mark()
    server.signal(signal.SIGUSR1)
    wait_reloads(t, server, 1)
    d.wait_event("disconnected", since=since, timeout=10)
    back = d.cmd("reconnect", timeout_ms=20_000)
    t.metric("reload_ms", (reload_times(server) or [0])[-1])
    t.metric("reconnect_ms", back["ms"])


def modes(d, pane) -> dict:
    s = d.screen(pane)
    return {k: s[k] for k in ("text", "alt_screen", "mouse_tracking", "cursor_visible", "cursor")}


@case(fixture="server_state")
def vim_keeps_its_screen_modes_and_buffer_across_a_reload_and_writes_it(t: T):
    """UC-061: vim with the mouse on: same screen and modes after a reload; mouse and :wq work."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane(80, 24)
    notes = t.scratch("vim") / "notes.txt"
    d.line(pane, f"vim -n -u NONE -N {notes}")
    d.wait_for(pane, r'"\S*notes\.txt" \[New\]', timeout=15)
    d.input(pane, ":set mouse=a nowrap\r")
    d.input(pane, "ifirst line\rsecond line\rthird line\x1b")
    d.wait_for(pane, r"(?m)^third line$")
    t.eventually(lambda: d.screen(pane)["mouse_tracking"], "vim turns mouse tracking on")
    t.eventually(lambda: "-- INSERT --" not in d.text(pane), "vim leaves insert mode")
    before = modes(d, pane)
    t.ok(before["alt_screen"] and before["mouse_tracking"] and before["cursor_visible"],
         f"vim runs on the alternate screen with the mouse on and a visible cursor ({before})")

    reload(t, server, d)
    after = modes(d, pane)
    t.check("the screen and modes are as they were", lambda: t.eq(after, before, "vim's screen, modes and cursor"))

    def mouse_moves_the_cursor():
        # Row 0 is "first line": a click on its sixth column puts the cursor there.
        d.cmd("mouse", pane=pane, action="press", button="left", x=5, y=0)
        d.cmd("mouse", pane=pane, action="release", button="left", x=5, y=0)
        d.input(pane, ":echo line('.') . ',' . col('.')\r")
        d.wait_for(pane, r"(?m)^1,6\s*$")
    t.check("a mouse click in vim moves its cursor after the reload", mouse_moves_the_cursor)

    def wq_writes():
        d.input(pane, "\x1bGo4th after the reload\x1b:wq\r")
        d.wait_for(pane, r"(?m)^\$ ?$")
        t.eq(notes.read_text(), "first line\nsecond line\nthird line\n4th after the reload\n", "what :wq wrote")
        s = d.screen(pane)
        t.eq((s["alt_screen"], s["mouse_tracking"]), (False, False), "the shell is back on the main screen, no mouse")
    t.check(":wq writes the buffer held across the reload", wq_writes)


@case(fixture="server_state")
def htop_keeps_its_modes_across_a_reload_and_answers_keys_and_the_mouse(t: T):
    """UC-061: htop: alternate screen, mouse on, cursor hidden after a reload; redraws, takes keys and F10."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane(120, 30)
    d.line(pane, "htop")
    d.wait_for(pane, r"Load average", timeout=15)
    t.eventually(lambda: d.screen(pane)["mouse_tracking"], "htop turns mouse tracking on")
    before = {k: v for k, v in modes(d, pane).items() if k in ("alt_screen", "mouse_tracking", "cursor_visible")}
    t.eq(before, {"alt_screen": True, "mouse_tracking": True, "cursor_visible": False},
         "htop: alternate screen, mouse on, cursor hidden")

    reload(t, server, d)

    def same_modes_and_redraws():
        first = d.screen(pane)
        t.eq({k: first[k] for k in before}, before, "htop's modes")
        t.has(first["text"], "Load average")
        d.cmd("sleep", ms=3000)
        later = d.screen(pane)
        t.ok(later["frames"] > first["frames"], f"htop redraws ({first['frames']} -> {later['frames']} frames)")
    t.check("htop keeps its modes and redraws", same_modes_and_redraws)

    def keys_and_mouse():
        # F1 opens help, Esc leaves it; a click on the header's "CPU%" sorts by it.
        d.key(pane, "f1")
        d.wait_for(pane, r"htop \d|Press any key|Help", timeout=10)
        d.key(pane, "esc")
        d.wait_for(pane, r"Load average", timeout=10)
        text = d.text(pane)
        header = next((y for y, row in enumerate(text.split("\n")) if "PID" in row and "CPU%" in row), None)
        t.ok(header is not None, "the process list header is on the screen")
        x = text.split("\n")[header].index("CPU%")
        d.cmd("mouse", pane=pane, action="press", button="left", x=x + 1, y=header)
        d.cmd("mouse", pane=pane, action="release", button="left", x=x + 1, y=header)
        d.key(pane, "f10")
        d.wait_for(pane, r"(?m)^\$ ?$", timeout=10)
    t.check("htop takes keys and the mouse and quits on F10", keys_and_mouse)
