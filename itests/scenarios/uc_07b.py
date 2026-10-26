"""UC-07B: the pane tools ($TILR_SHIMS: xclip, wl-paste, tilr) stay first on PATH when the
user's login profile sets PATH from scratch, in bash, zsh and fish; the user's own startup
files still run and the shell is still a login shell (`-bash`, `-zsh`, `-fish`). Another
shell (dash) starts as before, with $TILR_SHIMS set for the user's own fix.

Each user has its login shell in /etc/passwd and runs its own tilrd (the server reads the
shell from there); the case's driver connects over TCP with that server's token. A pane
with no command runs the user's login shell. bash goes through `--posix` and $ENV, zsh
through ZDOTDIR, fish through --init-command (crates/tilrd/src/shell.rs).
"""

import re

from tt import TMP, T, case

PROFILES = {
    "bash": {".bash_profile": "PATH=/usr/bin:/bin\nexport X=1\nPS1='$ '\nHISTFILE=/dev/null\n"},
    "zsh": {".zprofile": "path=(/usr/bin /bin)\n", ".zshrc": "export X=1\nPS1='$ '\n", ".zshenv": "export Y=1\n"},
    "fish": {".config/fish/config.fish": "set -gx PATH /usr/bin /bin\nset -gx X 1\n"
                                         "function fish_prompt; echo -n '$ '; end\nfunction fish_greeting; end\n"},
    "dash": {".profile": "PATH=/usr/bin:/bin\nexport X=1\nPS1='$ '\n"},
}
SHELLS = {"bash": "/bin/bash", "zsh": "/usr/bin/zsh", "fish": "/usr/bin/fish", "dash": "/bin/dash"}
# PATH joined by colons, and the shell's own pid, in each shell's words.
PATH = {"fish": "$(string join : $PATH)"}
PID = {"fish": "$fish_pid"}


def user_server(t: T, kind: str, env: dict | None = None):
    """A user whose login shell is `kind`, with the profile of PROFILES, running its own tilrd;
    a driver connected to it."""
    user = t.user(f"u{kind}", shell=SHELLS[kind])
    for name, text in PROFILES[kind].items():
        path = user.home / name
        r = user.run("sh", "-c", f'mkdir -p "$(dirname {path})" && cat > {path}', input=text)
        t.eq(r.code, 0, f"{name} written")
    return user, *connect(t, user, kind, env)


def connect(t: T, user, name: str, env: dict | None = None):
    """Start the user's own tilrd (as the user, with `env`) and connect a driver to it over TCP
    with its token; a pane running the user's login shell, once its prompt is up."""
    run = user.home / "tilr-run"
    senv = user.env(TILR_RUNTIME_DIR=run / "runtime", TILR_STATE_DIR=run / "state", TILR_CONFIG_DIR=run / "config",
                    **(env or {}))
    log = TMP / f"tilrd-{name}.log"
    t.spawn("runuser", "-u", user.name, "--", "env", "-i", *[f"{k}={v}" for k, v in senv.items()],
            "tilrd", "--listen", "127.0.0.1:0", log=log)
    # Found by its log: the pid of another user's listener is not shown to ss here.
    url = t.eventually(lambda: (m := re.search(r"listening on (ws://\S+)", log.read_text(errors="replace")))
                       and m.group(1), f"{name}'s tilrd listens", timeout=30)
    token = t.eventually(lambda: (run / "state/token").exists() and (run / "state/token").read_text().strip(),
                         "the server's token")
    d = t.driver(f"driver-{name}")
    d.connect(url, token=token)
    pane = d.shell_pane()
    return d, pane


def value(d, pane: str, expr: str) -> str:
    """What `printf` of `expr` prints in the pane's shell (bash, zsh, dash and fish alike)."""
    value._n = getattr(value, "_n", 0) + 1
    n = value._n
    d.line(pane, f"printf '<v%s>%s</v%s>\\n' {n} \"{expr}\" {n}")
    return d.wait_for(pane, rf"<v{n}>(.*?)</v{n}>", timeout=15)["groups"][1]


def argv0(d, pane: str, kind: str) -> str:
    """The shell's argv[0], from /proc."""
    return value(d, pane, f"$(tr '\\0' '\\n' < /proc/{PID.get(kind, '$$')}/cmdline | head -n 1)")


@case(spec=[
    "panes/the-tools-stay-first-on-path-after-the-login-profile#bash-with-a-profile-that-resets-path",
    "panes/the-tools-stay-first-on-path-after-the-login-profile#zsh-and-fish",
    "panes/the-tools-stay-first-on-path-after-the-login-profile#another-shell",
], timeout=180)
def the_tools_stay_first_on_path_when_the_profile_resets_it(t: T):
    """UC-07B: profiles that set PATH from scratch: bash, zsh, fish have $TILR_SHIMS first (once); dash as before."""
    for kind in ("bash", "zsh", "fish"):
        def check(kind=kind):
            user, d, pane = user_server(t, kind)
            shims = value(d, pane, "$TILR_SHIMS")
            t.ok(shims.endswith("/bin"), f"{kind}: TILR_SHIMS is set ({shims!r})")
            path = value(d, pane, PATH.get(kind, "$PATH")).split(":")
            t.eq(path, [shims, "/usr/bin", "/bin"], f"{kind}: PATH in a new pane")
            t.eq(value(d, pane, "$(command -v xclip)"), f"{shims}/xclip", f"{kind}: xclip is the server's")
        t.check(f"{kind}: the tools come first", check)

    def dash():
        user, d, pane = user_server(t, "dash")
        t.eq(value(d, pane, "$PATH"), "/usr/bin:/bin", "dash: PATH as the profile set it")
        t.ok(value(d, pane, "$TILR_SHIMS"), "dash: TILR_SHIMS is set for the user's own fix")
    t.check("dash starts as before", dash)


@case(spec=["panes/the-tools-stay-first-on-path-after-the-login-profile#the-user-s-files-still-run"], timeout=180)
def the_users_startup_files_still_run(t: T):
    """UC-07B: X=1 from the user's files, argv[0] -bash/-zsh/-fish, zsh's ZDOTDIR as it was (own or unset)."""
    for kind in ("bash", "zsh", "fish"):
        def check(kind=kind):
            user, d, pane = user_server(t, kind)
            t.eq(value(d, pane, "$X"), "1", f"{kind}: the user's file ran")
            t.eq(argv0(d, pane, kind), f"-{kind}", f"{kind}: a login shell")
            if kind == "zsh":
                t.eq(value(d, pane, "$Y"), "1", "zsh: the user's .zshenv ran")
                t.eq(value(d, pane, "${ZDOTDIR-unset}"), "unset", "zsh: ZDOTDIR is unset as it was")
                t.eq(value(d, pane, "${TILR_ZDOTDIR-none}"), "none", "zsh: nothing of the wrapper's left")
            if kind == "bash":
                t.eq(value(d, pane, "$(shopt -q login_shell && echo login)"), "login", "bash: shopt login_shell")
                t.eq(value(d, pane, "$(set -o | grep -c '^posix *on')"), "0", "bash: not in posix mode")
                t.eq(value(d, pane, "${ENV-unset}"), "unset", "bash: ENV is not left set")
        t.check(f"{kind}: the user's files run", check)

    def zsh_own_zdotdir():
        user = t.user("uzdot", shell=SHELLS["zsh"])
        zdot = user.home / ".config/zsh"
        r = user.run("sh", "-c", f"mkdir -p {zdot} && printf '%s\\n' 'export X=own' \"PS1='$ '\" > {zdot}/.zshrc"
                                 f" && printf 'export Y=own\\n' > {zdot}/.zshenv")
        t.eq(r.code, 0, "the user's ZDOTDIR files")
        d, pane = connect(t, user, "zdot", {"ZDOTDIR": zdot})
        t.eq(value(d, pane, "$X $Y"), "own own", "zsh reads the user's own ZDOTDIR")
        t.eq(value(d, pane, "$ZDOTDIR"), str(zdot), "ZDOTDIR is the user's")
        shims = value(d, pane, "$TILR_SHIMS")
        # Only the first directory: a whole PATH wraps on the pane's screen.
        t.eq(value(d, pane, "${PATH%%:*}"), shims, "and the tools come first")
    t.check("zsh with a ZDOTDIR of the user's", zsh_own_zdotdir)
