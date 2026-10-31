# tilrd

A terminal server: it runs shells in panes on a machine and serves them to clients over a
WebSocket. A client gets the screen as a grid of cells and sends keys back; panes keep
running when clients disconnect, and survive the server reloading itself into a new
version.

`tilrd` is the server, a service: `tilr` starts it, or a service manager, or the Tilr app;
nobody needs to run it by hand. `tilr` is the command a user types: the
[terminal client](#the-client-tilr) (tmux-style) and every other command (`tilr token`,
`tilr update`, `tilr cloud login`, `tilr ps`). Both are built from this repository.

Status: early (see [TODO.md](TODO.md)).

```text
crates/tilr-proto/        the wire protocol: proto/tilr.proto and packet framing
crates/tilr-screen/       terminal screen model (libghostty-vt) that produces frames
crates/tilrd/             the server binary `tilrd`
crates/tilr/              the client binary `tilr`: attaches to panes, and the user's commands
crates/tilr-driver/       a scripted client for the black-box tests in itests/
itests/                   black-box tests: real installs and servers in containers (Dagger)
scripts/release.sh        the Linux release archive in the CDN layout; --publish uploads it
scripts/package.sh        GitHub release archive of built binaries (release.yml)
scripts/linux.sh          a command (the test suite by default) in the Linux build container
scripts/install-tools.sh  the pinned Zig, protoc and cargo-nextest (CI, the container)
scripts/update-test.sh    live check of `tilrd update` on a real machine
```

One supervisor process accepts clients and routes packets. Each pane is its own process, a
child of the supervisor, which owns the pty, the shell and the screen model.

A pane opened without a command runs the user's login shell (the password database's, else
`$SHELL`, else `/bin/sh`) as a login shell, `argv[0]` = `-zsh` and the like, so it reads the
user's profile whatever environment the server was started in (a launchd agent gets the bare
system `PATH`). The pane gets `SHELL`, `TERM=xterm-256color`, `COLORTERM=truecolor`,
`TILR_PANE`, and `HOME`, `USER`, `LOGNAME` from the password database where the server's own
environment lacks them (an inherited `HOME` wins); launchd's `XPC_SERVICE_NAME` /
`XPC_FLAGS` and what systemd hands the server (`NOTIFY_SOCKET`, `LISTEN_FDS`,
`LISTEN_FDNAMES`, `LISTEN_PID`, `WATCHDOG_USEC`, `WATCHDOG_PID`) are dropped. A pane with a
command runs it as given, without a shell.

The server's tools for the panes (`xclip`, `wl-paste`, `wl-copy`, `tilr`; their directory is
`$TILR_SHIMS`) come first on `PATH`, also when a startup file sets `PATH` from scratch: zsh,
bash and fish start through small wrappers in `<runtime dir>/shell/` that run the user's
files as that login shell would and then put `$TILR_SHIMS` first (zsh: `ZDOTDIR` pointed
there for `.zshenv`, the user's own restored at once, the fix in a one-shot `precmd` hook;
bash: `-bash --posix` with `$ENV`, which leaves posix mode and reads `/etc/profile` and the
first of `~/.bash_profile`, `~/.bash_login`, `~/.profile`; fish: `--init-command`). Other
shells start as above; `PATH="$TILR_SHIMS:$PATH"` at the end of their profile does the same.
All servers of one user share these tools and wrappers; a starting server leaves those that
are already right alone and replaces the others by a rename, so the panes of a running one
never find a tool or wrapper missing. The last server to start decides which executable the
tools are.

## Install

Prebuilt binaries for macOS (arm64, x86_64) and Linux (x86_64, aarch64; glibc 2.36+) are
attached to each [GitHub release](https://github.com/tilr-dev/tilrd/releases) as
`tilr-vX.Y.Z-TARGET.tar.gz` with a `.sha256` next to it; the archive holds `tilrd`, `tilr`,
the licenses and this README.

From this repository (needs the [build prerequisites](#build)):

```sh
cargo install --path crates/tilrd --locked
cargo install --path crates/tilr --locked
```

**Linux, from the CDN.** The Linux releases are also on `https://cdn.tilr.dev`, where the
installer and `tilr update` look for them: one archive per platform,
`tilr-<version>-<platform>.tar.gz`, holding the server `tilrd` and its client `tilr`, with
its `.sha256`. The installer (`curl -fsSL https://tilr.sh | sh`; `tilr.sh` serves
`scripts/install.sh`, uploaded with each release) downloads it into a
temporary directory, checks it, unpacks it there and runs both programs, and only then puts
both in `~/.local/bin`; a failed check installs nothing. After installation it explicitly
requests reload. If that request fails, both new programs remain installed and the command
exits non-zero with an error saying reload could not be requested. It sets up no service and
edits no profile: it prints what to type next (`tilr`, `tilr service install` to start
tilrd at boot, `tilr token`, `tilr cloud login`) and, if `~/.local/bin` is not on `PATH`,
the line to add to `~/.profile`. By hand:

```sh
platform=linux-aarch64            # or linux-x86_64
version=$(curl -fsSL "https://cdn.tilr.dev/releases/$platform/latest")
curl -fsSL "https://cdn.tilr.dev/releases/$version/tilr-$version-$platform.tar.gz" |
  tar -xz -C ~/.local/bin tilrd tilr
tilr
```

**macOS application distribution.** The Mac app uses the ordinary `tilrd` installation.
Update the server with `tilrd update` or `tilr update` from a shell.

### Updating

In any shell on that machine:

```sh
tilr update
```

`tilr update` has the running server do it (it runs `tilrd update` as a child of the
server, in its environment), then reconnects and prints the version the server reloaded
into. It asks the CDN which version is ready for its platform, verifies the Ed25519 signature
of its channel manifest using the public key embedded in `tilrd`, downloads its archive and
checks its SHA-256 against that authenticated manifest, unpacks it into a directory next to the executable, checks that
both programs run, and renames them over the installed ones: `tilr` (next to `tilrd`) first,
`tilrd` last; if `tilrd` cannot be put in place, the old `tilr` is put back. Both programs are
synced to disk before the renames and the directory after them, so `updated …` means the
release is on disk (a power loss after it does not leave an empty executable). A running server
then [reloads itself](#reloading-without-closing-shells) and keeps every shell. `--cdn URL`
(or `TILR_CDN`) names another place to fetch releases from. If the reload request fails
after installation, the new programs remain installed, the command exits non-zero and
reports the request failure without claiming reload succeeded.

Missing or invalid signed metadata fails before an archive is downloaded or either program
changes. There is no fallback to the unsigned `.sha256` file. The manifest binds the version,
channel (`stable` or `beta`), platform, archive basename and checksum; a different CDN or
runtime environment cannot replace the embedded trust key. Signatures authenticate releases, including intentional
rollbacks; they reject manifests from another channel before requesting an archive, but do
not prevent replay of a previously signed release within the requested channel.

A legacy `app-managed.json` next to `tilrd` is ignored by the updater and remains unchanged.

`tilr version` prints what builds the client and the running server are: the package
version, the commit and the platform, e.g. `0.1.0-26449dd (linux-aarch64)` (`tilr
--version`, `tilrd --version`: one each). The running server also reports its
configured update channel on a separate `tilrd update channel: stable` (or `beta`)
line, after `tilrd <version> at <target>`. Older servers that omit the channel
produce no channel line.

## Quickstart

```sh
tilr                              # starts tilrd if none runs, attaches to a pane
tilr token                        # the access token a client on the TCP port must present
tilr ps                           # the panes of the running server
```

A client on this machine talks to the server over its [Unix socket](#running-tilrd). A
client elsewhere connects to `ws://127.0.0.1:7890`, sends `Hello` with the
[access token](#who-may-connect), and gets `HelloAck`; see [Protocol](#protocol). To reach a
server on another machine, forward its port (`ssh -L 7890:127.0.0.1:7890 my-server`) or link
it to [tilr Cloud](#tilr-cloud) for a tunnel.

## The client: tilr

```sh
tilr                        # attach to the pane used last, or make one
tilr new [-d] [--cwd DIR] [-- CMD…]   # a new pane (-d: print its id, do not attach)
tilr attach [PANE]          # a pane by its id or the start of it (tilr a)
tilr ls [-q]                # the panes; * marks the one used last (-q: full ids)
tilr kill [PANE]            # close a pane: the one this runs in, else the one used last
tilr kill-server            # stop the server: shells end, socket and lock go
tilr ps                     # the panes with the pids of their processes
tilr version                # the client's and the server's builds
tilr token | update | cloud login|status|logout   # run in the server, see below
tilr deploy [--service] [--archive FILE] [--cdn URL] user@host  # install over SSH
tilr service install|uninstall                    # start tilrd at boot, see below
tilr edit FILE | open URL   # inside a pane: ask the clients watching it to open it
```

`tilr deploy user@host` uses your SSH config and agent to install both programs in
`~/.local/bin` on the remote and start tilrd. It prints `ssh -t user@host '~/.local/bin/tilr'`
for a terminal, `ssh -N -L 7890:127.0.0.1:7890 user@host` for the app, and the remote
access token. `--service` also installs the remote user's boot service. Releases
come from `--cdn URL`, `TILR_CDN`, or `https://cdn.tilr.dev`; `--archive FILE` uses a
local `tilr-<version>-<platform>.tar.gz` with its `.sha256` alongside it. Supported
hosts are Linux x86_64/aarch64 and macOS arm64 (macOS requires a local archive).
An existing server reloads into the new release and keeps its panes. Failures before
installation and startup complete restore the previous remote files. If the subsequent
reload request fails, deploy keeps both new programs, exits non-zero with an explicit
request error and prints no connection instructions or token.

**The tmux model.** `tilr` talks to the server of this user on this machine over a Unix
socket. With no server running it starts one: `tilrd` next to the `tilr` executable, else on
`PATH`, else `$TILRD`; detached (a grandchild in a session of its own, no controlling
terminal, stdin from `/dev/null`, its log in `tilrd.log` next to the socket). Two `tilr`
started at once on an empty machine start one server: the first to take the socket's lock
starts it and hands it the lock (`--lock-fd`), the other waits for its socket. The server
outlives the terminal: a hung-up SSH session takes `tilr` with it, not the server or its
shells. `tilr ls`, `kill` and `attach PANE` start nothing.

**Attached.** The terminal is in raw mode on the alternate screen and shows the pane, sized
to the terminal (a resize follows). **Ctrl+] is the prefix**: Ctrl+] d detaches, Ctrl+]
Ctrl+] sends Ctrl+] itself; another key after it is sent as it is. Keys go as `KeyInput`,
with `code` (the physical key) for the keys a terminal names (Enter, Tab, the arrows,
F1…F12); a character key comes from the terminal as the character the layout made, Ctrl
folded in (Ctrl+С on a Russian layout arrives as ^C), so its `code` stays empty. Pastes go
as `Paste` (bracketed), the mouse only while the program in the pane asks for it: `tilr`
captures the mouse only then, so otherwise the wheel is the outer terminal's own (its
scrollback, or arrow keys on the alternate screen). The pane used last is the one with the
latest `PaneInfo.used_at_ms` (a subscribe, or input).

**Disconnects.** The client has one `Hello.client_id` (a UUID) for as long as it runs. A
connection that drops (a reload of the server, the network) is made again with it 0.1 s
after the loss, then after waits from 0.1 s doubling to 2 s; the bottom row says
`disconnected …, reconnecting…` meanwhile. Input stays queued until the pane acknowledges
application to its PTY and is replayed after reconnect without applying duplicates. The
queue holds at most 4 MiB or 4096 messages; a full queue rejects new input with a visible
`input buffer full` error. An attempt that connects but loses the connection (or gets no answer) before the pane
is listed and subscribed is a failed attempt like a refused one, and Ctrl+] d detaches at
once also while an attempt waits. It pings every 5 s, on the Unix socket as over TCP, and
takes 15 s of silence as a dropped connection. A pane gone after the reconnect, or a local
server that stopped (socket gone, lock free), ends the attach (`[tilrd stopped]`). A
connection closed as superseded (close code 4000: another connection said `Hello` with this
`tilr`'s client id) is not made again: `tilr` restores the terminal, prints `[superseded by
another connection of this client]` and exits 0, rather than take the pane back from the
other in a loop. SIGTERM, SIGHUP or SIGINT (`kill -INT`; Ctrl+C is a key for the pane)
restore the terminal (cooked mode, cursor, main screen) and end `tilr` by that signal,
printing nothing; the pane goes on.

**Copies.** `tilr` lists `clipboard-write` in `Hello.features`: text a program in the
attached pane copies (see [Copying from a pane](#copying-from-a-pane)) is written to the
terminal as `ESC ] 52 ; c ; <base64> ESC \`, for the terminal to put on the clipboard
of the user's machine as its settings allow; other types (an image) are not passed on.

**Which server.** `--socket PATH` / `TILR_SOCKET`, or `--connect ws://HOST:PORT` /
`TILR_CONNECT` with `--token` / `TILR_TOKEN` for a server's TCP port (not started from
here). Without a token given, a server on the loopback (`localhost`, `127.0.0.0/8`, `::1`)
gets the one of this machine's server (the state directory's `token`); any other host gets
none: this machine's token never leaves it, so give that server's own (`--token`,
`TILR_TOKEN` or `token` in `client.toml`). Else `~/.config/tilr/client.toml`
(`$XDG_CONFIG_HOME`, or `TILR_CONFIG_DIR`):

```toml
connect = "ws://10.0.0.5:7890"   # or: socket = "/path/tilrd.sock"
token = "…"
```

**Commands run in the server.** `tilr token`, `tilr update [--cdn URL]` and `tilr cloud
login|status|logout` work on what the server owns (its token, its executable, its link to
Cloud): `tilr` asks the running server, starting it if needed, to run its own executable
with those arguments (`ExecStart` with `server_command`, taken on the Unix socket only, with
exec on or off) and shows the output as it comes; Ctrl+C cancels. There is no offline mode:
a server is started for them. Over `--connect` they are refused: run `tilr` on that machine.

**In a pane.** The server puts its own `tilr` first on the `PATH` of every pane (a link to
`tilrd`, see [Files and `tilr edit`](#files-and-tilr-edit)): it answers `tilr edit` and
`tilr open` (to the clients watching the pane, over `$TILR_SOCK`) and hands every other
command to the next `tilr` on `PATH`, this client. The client answers `edit` and `open` the
same way, for a pane whose profile reset `PATH`. Panes get `TILR_SOCKET`, so `tilr ls` in a
pane lists its own server's panes; `tilr attach` refuses the pane it runs in.

## Running tilrd

`tilrd` is a service; `tilr` starts it as `tilrd --socket PATH --lock-fd N`. Run by hand or
by a service manager it stays in the foreground and logs to stderr:

```sh
tilrd                                 # the Unix socket and TCP 127.0.0.1:7890
tilrd --listen 0.0.0.0:7890           # TCP on every address (the token is still asked)
tilrd --no-listen                     # the Unix socket only
```

**Log level.** `RUST_LOG` sets the log filter in tracing's directive syntax: `debug`,
`tilrd=debug,info`, `tilrd::supervisor=trace`. Unset or empty, the server logs at `info`; a
value that does not parse is ignored with one line `RUST_LOG ignored: <reason>` and the
server logs at `info`. It applies to the supervisor and the pane processes alike. `RUST_LOG`
stays in the server's environment, so a reload keeps the level and the shells in panes
inherit the variable like any other.

**TCP.** On by default, on loopback: `127.0.0.1:7890` (`--listen ADDR` or `TILR_LISTEN`),
where the Tilr app, an SSH tunnel (`ssh -L 7890:127.0.0.1:7890`) and the
[Cloud tunnel](#tilr-cloud) reach the server with the access token. `--listen [::]:7890` is
both IPv4 and IPv6 where the system allows dual stack. `--no-listen` turns TCP off. A port
that is taken stops the server with an error that says so (exit status 1), so a service
manager shows it, except in a server `tilr` started: that one warns in its log and serves
the Unix socket alone, without a tunnel, rather than another port nobody would know of. A
Tilr app's server or another user's may hold 7890; `tilr` on the machine works either way.
A reload keeps what the server had: its TCP socket, or none. A connection on the port that
has not finished its WebSocket handshake (or SOCKS request) within 10 s is closed, so
connections that say nothing hold no descriptor longer; a [metrics](#metrics) request whose
head has not come within 10 s gets `400`. A main-protocol
connection (on the port or the Unix socket) that has not sent `Hello` within 10 s of its
WebSocket handshake is closed with close code 4002 (`timeout`), whatever `--client-timeout`
is (also 0); the log says `client sent no Hello in 10s`. There is no limit on their number
otherwise.

**Sockets.** The Unix socket is always on unless `--no-socket` (not with `--no-listen`). The
socket is `--socket PATH`, else `$TILR_SOCKET`, else `tilrd.sock` in the runtime directory:
`$TILR_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/tilr`, else `tilr-<uid>` in the temporary
directory (`$TMPDIR` on macOS, `/tmp` on Linux). The directory is made 0700 (and must be the
user's), the socket 0600, and the server checks each connection's peer uid (`SO_PEERCRED`,
`getpeereid`) is its own: on the socket no access token is asked. `tilr` checks the other
way round before it says anything: the runtime directory must be a directory (not a link)
of the user's with nothing for group and others, or `tilr` neither connects there nor
starts a server (another user may have made `/tmp/tilr-<uid>` first), and whatever the
socket's path, a server whose peer uid is not the user's is left at once, not a byte sent
(`socket … is served by another user (uid N)`). The lock file and `tilrd.log` are opened
without following a symbolic link (`O_NOFOLLOW`). A path longer than a
socket can be bound at (107 bytes on Linux, 103 on macOS) is an error that says so. One
server per socket: it holds `tilrd.sock.lock` (`flock`, with its pid in it) for as long as
it runs, a reload included. What is found at the path when it starts:

- a socket nobody accepts on and whose lock is free (a server killed with SIGKILL left it):
  removed and replaced;
- a socket some other program listens on, a socket of another user, a file that is not a
  socket: an error, nothing removed.

A reload keeps the socket and the lock (their descriptors go to the successor as the TCP
socket's does). Stopping removes the socket and the lock file.

**The Tilr app is unaffected.** It runs `tilrd --listen 127.0.0.1:7890 --config …` and uses
its TCP port and `tilrd token`, `ps --url`, `state-formats` and `--version`, which stay
(hidden from `--help`). With TCP on and no `--socket`, a socket that cannot be taken
(another tilrd, `tilr`'s, serves it; the path is too long) is only a warning: that server
serves TCP alone. If the app's server took the socket first, `tilr` attaches to it. A server
`tilr` started first holds 7890 in turn (same user, same token): the app's own server then
cannot listen there.

| Flag | Default | Meaning |
|---|---|---|
| `--listen ADDR` | `127.0.0.1:7890` (`TILR_LISTEN`) | The TCP address of the WebSocket, e.g. `0.0.0.0:7890`, `[::]:7890` |
| `--no-listen` | | No TCP port: the Unix socket only |
| `--socket PATH` | see above | The Unix socket; must work when named |
| `--no-socket` | | No Unix socket (not with `--no-listen`) |
| `--allow-origin ORIGIN` | the configuration file's list, else `tilr://app` (the Tilr app's page), `tilr://tilr` (its terminal app) and the web client's | Browser origins allowed to connect; repeatable |
| `--config PATH` | `~/.config/tilr/server.toml` (`$XDG_CONFIG_HOME/tilr/server.toml`), if there | The [configuration file](#the-configuration-file) (also `TILR_CONFIG`) |
| `--no-auth` | | Do not ask clients for the [access token](#who-may-connect) |
| `--cloudflared PATH` | `cloudflared` | The `cloudflared` to run for a [tunnel](#tilr-cloud) (also `TILR_CLOUDFLARED`) |
| `--no-clipboard` | | Turn the [shared clipboard](#pasting-images-the-shared-clipboard) off |
| `--clipboard-max SIZE` | `20MB` | Largest clipboard upload (`512K`, `20MB`, `1G`) |
| `--clipboard-ttl SECS` | `300` | How long uploaded content stays; `0` = until replaced |
| `--client-timeout SECS` | `30` | Drop a client that stays silent, or does not answer the server's pings, this long; `0` = never |
| `--socks` | the configuration file's `socks`, else off | Serve the [SOCKS proxy](#socks-proxy) on the same port |
| `--no-files` | | Do not offer the [files of this machine](#files-and-tilr-edit) (also `[files] enabled = false`) |
| `--no-agent` | | Turn [SSH agent forwarding](#ssh-agent-forwarding) off (also `agent = false`) |
| `--no-exec` | | Do not let clients [run programs](#running-programs-exec) outside a pane (also `exec = false`) |

Signals: `SIGTERM` and `SIGINT` stop the server and end its shells (`tilr kill-server`
sends SIGTERM and waits for the server and its pane processes to be gone). Each pane
process, when the server stops or dies, does what a closed terminal does: SIGHUP to the
shell's process group and to the terminal's foreground group (the job running); what of
those two groups ignores SIGHUP is killed 2 s later, and the server waits for that. A job in
the background that ignores SIGHUP (`nohup cmd &`) is left, as after an SSH hangup;
closing a pane (`ClosePane`, `tilr kill`) ends the same two groups the same way. `SIGUSR1`
[reloads](#reloading-without-closing-shells) it. `SIGHUP` is ignored: a server started from
a terminal (`tilrd &` in an SSH session) outlives it, and so do its panes, each in a session
of its own. Its log then goes nowhere; redirect stderr to keep it. A signal that arrives
while the server starts (its socket or port already served, its handlers not yet in place)
is noted and handled once it is up: SIGTERM or SIGINT a clean stop (socket and lock file
removed, exit 0), SIGHUP ignored, SIGUSR1 one reload.

### Who may connect

On the Unix socket: the user, as above, without a token. On the TCP port a client must send
the server's access token in `Hello.token`. The server makes the token the first time it
runs and keeps it in `$XDG_STATE_HOME/tilr/token` (`~/.local/state/tilr`, a directory made
0700; one that is there already is set to 0700 at start, and one of another user's is
refused), readable by the user only. To see it, on that machine:

```sh
tilr token
```

A client without the token, or with another, gets an `Error` and the connection is closed.

`--no-auth` turns the check off: then anyone who can reach the port gets a shell. A page in
a browser must, with or without the token, come from an allowed origin: `--allow-origin`,
else the configuration file's list, else the built-in one. The origin is checked on the
Unix socket too: a handshake there that carries an `Origin` header not on the list is
refused (403) as on the port; one without the header passes.

### The configuration file

What a machine's operator sets once lives in `~/.config/tilr/server.toml`
(`$XDG_CONFIG_HOME/tilr/server.toml`, or `--config PATH` / `TILR_CONFIG`), read when the
server starts and by `tilrd update`. Server settings take effect on restart
(including a reload after `tilr update` or SIGUSR1); the update channel takes effect
on the next update. A missing default file is an empty configuration; one that does
not parse, or names an unknown key, prevents startup and updating.

```toml
# Release channel read by tilrd update: "stable" or "beta". Default: "stable".
update_channel = "stable"

# Browser origins allowed to connect. The Tilr app serves each of its apps
# from tilr://<name>/ (the name from its config.yml, lower-cased); the built-in
# list has tilr://app, tilr://tilr (the terminal) and the web client's, so add
# your own apps here. Replaces the built-in list; --allow-origin replaces this one.
allow_origins = ["tilr://app", "tilr://tilr", "tilr://my-notes"]
# Serve the SOCKS proxy, as --socks does. Off when unset.
socks = true
# Forward the SSH agent of the clients to the panes. On when unset;
# false does what --no-agent does.
agent = true
# Let clients run programs outside a pane (Exec). On when unset; false does
# what --no-exec does.
exec = true

# The files protocol.
[files]
# Off: clients cannot see or touch files, as --no-files does. Default on.
enabled = true
# The directories clients may go into. Default: the home directory.
roots = ["~", "/srv/www"]
# Clients may look but not change anything. Default off.
readonly = false
# Largest file content one read returns or one write takes: bytes, or a
# size like "64MB". Default 64MB.
max_bytes = "64MB"
```

## Starting at boot

Optional, and only when asked: nothing starts tilrd at boot but this.

```sh
tilr service install      # start tilrd at boot, and again if it fails
tilr service uninstall    # remove what install added
```

The service runs tilrd for the socket `--socket PATH` / `TILR_SOCKET` names, else the
default one; `socket` in `client.toml` is not read here.

- **Linux with systemd and lingering**: a user unit, `~/.config/systemd/user/tilr.service`
  (`$XDG_CONFIG_HOME`): `Type=notify` (tilrd says `READY=1` on `$NOTIFY_SOCKET` once it
  listens), `ExecStart=/bin/sh -lc 'exec "…/tilrd"'` (a login shell, so the panes get the
  environment of `~/.profile`, as over SSH), `Restart=on-failure`; `systemctl --user enable`
  (with `--now` unless a tilrd runs). Lingering (the user's manager runs from boot, not only
  while logged in) is turned on with `loginctl enable-linger` if it is off; the unit notes
  that, and uninstall turns it off again.
- **No systemd user manager, or lingering refused**: a line in the user's crontab,
  `@reboot /bin/sh -lc '… exec "…/tilrd" --socket … >>…/tilrd.log 2>&1' # tilr service`, and
  tilrd is started now if none runs. The output says which was chosen. Neither systemd nor
  `crontab`: an error, nothing written.
- **macOS**: a launchd agent, `~/Library/LaunchAgents/sh.tilr.tilrd.plist` (`RunAtLoad`,
  `KeepAlive` on failure, the socket named, the log in `~/Library/Logs/tilrd.log`), loaded
  with `launchctl bootstrap gui/<uid>`.

Install replaces only a unit or plist it wrote itself (they carry `tilr service`): over one
of your own it fails with `… was not written by `tilr service install`: move it away to
install` and changes nothing. `crontab` runs in the C locale, and a user without a crontab
yet is recognized from cron's and busybox's messages.

A tilrd that already runs (one `tilr` started) is left alone: the service takes over from
the next boot. Uninstall removes the unit, the crontab line (the whole crontab if install
made it) or the agent, and nothing it did not write; it stops nothing, so the running server
and its shells go on until `tilr kill-server`.

## tilr Cloud

Optional. Cloud lists the machines of a user, says which are online, and gives each a
tunnel: a name on the internet that leads to the server's TCP port (a server on its Unix
socket alone, `--no-listen`, gets no tunnel), so a phone can reach a machine that accepts no
incoming connections. What goes through the tunnel does not pass through Cloud.

```sh
tilr cloud login      # prints a link and a code to confirm in a browser
tilr cloud status
tilr cloud logout     # Cloud forgets the machine and removes its tunnel
```

`cloud login --name NAME` names the machine there (the host name by default).

A linked server keeps one outgoing connection to Cloud. When Cloud tells it to, it runs
`cloudflared` (which must be installed) with a configuration it writes itself: the tunnel
leads to the server's own port and nowhere else. A running server notices a login or a
logout within a few seconds. `cloudflared` runs in a process group of its own (a terminal's
SIGHUP or Ctrl-C sent to the server's group does not reach it) and does not outlive the
server: a stop or a reload ends it (SIGTERM, then SIGKILL after 3 s); on Linux it gets
SIGTERM when the server dies, even of SIGKILL; elsewhere a server killed that way leaves it
running until the next server starts on that runtime directory, which stops the one named
in `cloudflared/pid` there if it still runs that directory's `config.yml`.

What Cloud can and cannot do:

- It can open and close the tunnel, and it sees whether the server is online, its version
  and platform.
- It cannot reach a pane: nothing it sends is passed on to one, and it never sees the access
  token. A client that comes through the tunnel needs the token like any other.
- A server started with `--no-auth` takes no tunnel.
- The tunnel is in Cloud's Cloudflare account, and TLS ends at Cloudflare: whoever controls
  that account can read what goes through. End-to-end encryption between client and server
  is not there yet.

The link (`cloud.json`, next to the token) holds a token of the machine's own for Cloud; it
is not the access token. `--url` or `TILR_CLOUD` names another Cloud than
`https://wss.tilr.dev`.

## SOCKS proxy

Off unless the server runs with `--socks` (or `socks = true` in the configuration file).
Then the server is also a SOCKS5 proxy (RFC 1928) on its own port, so a browser on another
machine opens what listens there, a dev server on `127.0.0.1:3000` say. CONNECT only; BIND
and UDP ASSOCIATE get "command not supported".

Names are resolved on the server ("remote DNS"). `localhost` and every name under it
(`pb.localhost`, `anything.localhost`, RFC 6761) never go to DNS: they reach services bound
on this machine's loopback, 127.0.0.1 first, then `[::1]`. A browser connects to
`localhost` and `127.0.0.1` itself rather than through a proxy, so a page asks for
`pb.localhost:3000` instead.

Two ways in:

- **`GET /socks`, a WebSocket**: the way through a [tunnel](#tilr-cloud), which carries HTTP
  only. The handshake shows the access token: `Authorization: Bearer <token>`, or
  `?token=<token>` for a page, which cannot set headers on a WebSocket. A browser page must
  come from an allowed origin, as for the main protocol. Without the token: 401; disallowed
  origin: 403; proxy off: 404. After that, the client's binary messages are the SOCKS bytes
  it sends and the server's are the ones it gets back, any sizes; SOCKS itself asks for no
  authentication (a username and password, if offered, are taken unread). A text message
  closes the WebSocket with 1003. A WebSocket cannot be half closed: once the target is done
  sending, the server closes it with 1000. The server pings it like the main protocol's
  clients (`--client-timeout`).
- **Raw SOCKS5** on the same port, for `ssh -L 1080:127.0.0.1:7890` and the like: a
  connection whose first byte is 5 is a SOCKS client, which must use username/password
  authentication (RFC 1929) with the access token as the password, any username. A server
  started with `--no-auth` asks for none.

```sh
ssh -L 1080:127.0.0.1:7890 my-server
curl --socks5-hostname "x:$(ssh my-server tilr token)@127.0.0.1:1080" http://pb.localhost:3000
```

The token gives what it gives through the main protocol: everything this user can do on the
machine, here any TCP connection from it. The raw mode listens wherever `--listen` does,
127.0.0.1 by default. Paths other than `/socks` are the main protocol, as before. Sessions
end when the server reloads.

## SSH agent forwarding

What `ssh -A` does, over the tilr protocol: `ssh`, `git` or `ssh-add` in a pane use the SSH
agent of the user's computer, the client's side, and the keys never leave it. On unless the
server runs with `--no-agent` (or `agent = false` in the configuration file).

- Every pane gets a Unix socket of its own, `agent/<pid>/<pane id>.sock` in the runtime
  directory (`$XDG_RUNTIME_DIR/tilr`; on macOS `~/Library/Application Support/tilr`;
  `/tmp/tilr-<uid>` where that path is too long for a socket; `<pid>` the server's, so two
  servers of one user keep theirs apart), and `SSH_AUTH_SOCK` pointing at it. The
  directories and the socket are the user's only. A [reload](#reloading-without-closing-shells)
  makes the socket again at the same path (the pid stays), so a shell's `SSH_AUTH_SOCK` stays
  good; the socket goes when the pane does, the directory when the server stops. A program of
  another uid is refused, even where the permissions were opened by hand. The sockets a server
  killed with SIGKILL left behind are removed when the next one starts: the `agent/<pid>/`
  directories of pids that are no longer running (a running server's are not touched).
- A client that can serve the agent says `"agent"` in `Hello.features`; the server lists
  `"agent"` in `HelloAck.features` when forwarding is on.
- A program that connects to the socket gets a stream to one client: of the clients watching
  the pane that said `"agent"`, the one that subscribed last. The client gets `StreamOpen`
  (kind `"ssh-agent"`, the pane's id); the bytes go both ways as `StreamData` until either
  side sends `StreamClose`. The server only carries them.
- With no such client attached, the connection is closed at once: `ssh` finds an agent with
  no keys and goes on to its other ways in. A client that goes away takes its streams with
  it; so does a reload.
- A client has at most 64 streams open; a program sending faster than the client takes is
  held up (the server reads from it only as its client's [queue](#protocol) has room, 4 MiB
  for all its streams and file reads), a client sending faster than the program reads loses
  the stream.

As with `ssh -A`: while a client with the feature watches a pane, every process of the user
on the server's machine that finds a socket can use the agent, for as many signatures as it
asks. The agent on the client's side asks for each signature if it is set up to (keys added
with `ssh-add -c`, say); otherwise turn forwarding off on servers you do not trust.

## Running programs (Exec)

A client can run a program on the server's machine without a pane or a shell, as `ssh host
cmd` would without the shell, and get its stdout and stderr apart and how it ended. On
unless the server runs with `--no-exec` (or `exec = false` in the configuration file); the
server lists `"exec"` in `HelloAck.features`. With exec off, an `ExecStart` is answered with
`ExecStarted` carrying `error` `exec is turned off`. It takes the same access token as
everything else: a client that may open a shell may run a program.

- `ExecStart` names the program and its arguments (`argv`), run as they are: no shell,
  nothing expanded; `argv[0]` is looked up in the server's `PATH`. Optionally a directory
  (`cwd`, else the home directory), `NAME=value` entries added to the server's environment
  (less the service-manager variables a pane does not get either, which `env` may still set),
  `stdin`, and a time limit (`timeout_ms`). The answer is `ExecStarted` with the exec's id
  and the program's pid, or with why it did not start (no such program or directory, more
  than 64 execs of the client running). `ExecStarted` and `ExecInputAck` are never dropped:
  a client whose queue cannot take one is dropped as too slow (close code 4001).
- The output comes as `ExecOutput` (stream 1 stdout, 2 stderr; at most 64 KiB each, in
  order per stream), then `ExecExit`: the exit code, or the signal that killed the program,
  and whether it was for the time limit (`timed_out`) or `ExecCancel` (`cancelled`). Both go
  by the [second lane](#protocol): a client that takes them slowly holds the program up,
  blocked writing to its pipe, and the server's memory stays flat. `ExecExit` is always the
  exec's last message.
- `ExecInput` feeds stdin and closes it (`eof`). The server acknowledges what the program
  took with `ExecInputAck` (bytes in all); a client keeps at most 4 MiB ahead of that, and
  one that sends more has the exec killed (`reason` `"input overrun"`).
- `ExecSignal` sends a signal to the program's process group, as a terminal does for Ctrl+C;
  `ExecCancel` kills it at once.

The program leads a process group of its own, and nothing of it outlives the exec: the whole
group is killed (SIGKILL) on the time limit, on `ExecCancel`, and when the program exits (a
background child that keeps the output open ends with it; the group is killed before the
program is reaped, so its id cannot have passed to another group). A process that left the
group (`setsid`) and keeps the output open is waited for at most 1 s, and read for at most
1 MiB more, after the group is gone; a cancel or the time limit ends that wait at once.
Execs are not panes and do not
survive a reload: it kills them and their groups, and a client still connected gets
`ExecExit` with `reason` `"reload"` before its connection goes (`"shutdown"` when the server
stops), after the output it was sent; for an exec that does not report within a second, an
`ExecExit` with only the reason. A client whose connection ends takes its execs with it. On Linux a program is also
killed should the server die without doing it (`PR_SET_PDEATHSIG`); its children then go
only if it had them in its group. The log has each start and end with the client's id.

## Files and `tilr edit`

The files protocol (feature `"files"`: `FsList`, `FsStat`, `FsRead`, `FsWrite`, `FsMkdir`,
`FsRename`, `FsDelete`, `FsWatch`, `FsUnwatch`; see tilr.proto) lets a client look at and
change the files of the server's machine, for a file browser and an editor. Only the
directories in `[files] roots` of the [configuration file](#the-configuration-file) are
served, the home directory by default; a path that leads out of them, by `..` or through a
symbolic link, is refused. With `readonly = true` nothing can be changed. With
`enabled = false` or `--no-files` the feature is not offered at all. One read returns, and
one write takes, at most `max_bytes` (`HelloAck.fs_max_bytes`). Names that are not UTF-8
are carried as [bytes](#file-names-that-are-not-utf-8). A write goes to a temporary file
next to the target, synced to disk and renamed over it, and the directory is synced after
the rename: a successful `FsAck` means the data and the name are on disk; a failed write
leaves the file as it was.

Host events (feature `"host-events"`) go the other way: a program in a pane asks the client
to open something. Every pane has `tilr` on its `PATH` (a link to `tilrd`, which answers to
that name; `tilrd edit` / `tilrd open` do the same), and `$TILR_PANE`, `$TILR_SERVER` (the
server's id) and `$TILR_SOCK` (the server's socket) in its environment:

```sh
tilr edit notes.md               # open the file in the client's editor
tilr open https://example.com    # open a URL; --kind and `-- args` say more
```

The server passes the request on as `HostEvent` (with an absolute path) to every client
watching that pane that announced `"host-events"`. Before it writes the request, `tilr`
checks who serves `$TILR_SOCK` (`SO_PEERCRED` / `getpeereid`): a socket of another user
gets not a byte, and `tilr` exits 1 with `cannot reach the tilr server: socket … is served
by another user (uid N), …`. With nobody to hear it, `tilr` says
"no tilr client attached" and exits with 1; outside a pane, "not inside a tilr pane". Other
`tilr` commands go to the next `tilr` on the `PATH`: the [client](#the-client-tilr).

The host socket is `<server_id>.sock` in the runtime directory (under `/tmp/tilr-<uid>/`
when that path is too long for a socket), 0600 in a 0700 directory. Two servers of one user
share the state directory and so the server id: a server that finds another live server
at `<server_id>.sock` (it connects, as tmux's client does; the live server closes such a
probe without a warning) uses `<server_id>-<pid>.sock` instead, and keeps it across a reload
(same pid). A stopping server removes its socket only while the file is still the one it
bound, and a starting one removes the `<server_id>-<pid>.sock` of pids no longer running.

## Telling panes apart

A pane is named by a UUID the server makes up when the pane is created. It names that pane
and no other: not on another machine, and not after the server is restarted, when a number
would be handed out again.

The id is the same everywhere a pane shows up:

- in the protocol, as `pane_id`;
- in the process list: the pane process runs as `tilrd pane --id <id>`, followed by how the
  pane was started (`--cols` and `--rows` unless 80×24, `--cwd`, the command), then
  ` [shell <pid> in <cwd>]`. While another process group is in the foreground, the suffix
  is ` [shell <pid> in <cwd>: <command>]` (up to 80 bytes of the command line).
  The directory follows OSC 7; foreground changes appear within a second, including after
  reload. A title longer than the available argument/environment area is cut. If that
  area cannot hold the start arguments and the start of the suffix, the original arguments
  remain and the pane logs `pane title: no room` once. The shell is the pane's child;
- inside the pane, as `$TILR_PANE`;
- in `tilr ps`, which asks the running server for its panes (`tilr ls` shows the first 8
  characters, which `tilr attach` and `tilr kill` take):

```
$ tilr ps
PANE                                      PID     SIZE  VIEWERS  CWD  COMMAND
3b750286-ae31-4cc4-9edd-3f314fb00d57     8193    80x24        1  /home/me  (shell)
d88b738a-fb48-41a5-bd4c-2050fdf33538     8219   120x40        0  /tmp  htop
```

`PID` is the pane process. `--connect ws://HOST:PORT` asks a server's TCP port instead.

A server has an id of its own too, in `HelloAck.server_id`: a UUID made the first time it
runs on a machine and kept in `$XDG_STATE_HOME/tilr/server-id` (`~/.local/state/tilr`), so
it is the same after a restart.

## Pane activity

Every greeted client receives `PaneActivity { pane_id, kind, at_ms }`, whether
or not it watches the pane. `kind` is `BELL` or `COMMAND_FINISHED`; `at_ms` is
Unix time in milliseconds. The client decides whether to notify.

Bells come from the terminal parser: a BEL ending an OSC sequence is not a bell.
A pane reports at most one bell per second; extra bells are dropped, not deferred.
Command completion follows the pty foreground process group returning to the
shell's, checked after output reads and once a second. It needs a shell with job
control. A command that runs and ends between checks without output can be missed.

A command still in the foreground after a reload remains tracked. The bell window
restarts after a reload. Reconnecting clients receive later events only; there is
no replay.

## Pasting images: the shared clipboard

A program in a pane cannot reach the clipboard of the user's computer. When it is asked to
paste an image it reads the clipboard of the machine it runs on, usually by running `xclip`
or `wl-paste`, and over a plain SSH session finds nothing there.

The server gives that machine a clipboard the client can fill:

1. The user pastes in the client. The client uploads the image with `ClipboardPut` (chunks
   of about 256 KB), then sends the paste key, Ctrl+V, to the pane.
2. The server stores the content, one clipboard for all panes.
3. The program in the pane runs `xclip` or `wl-paste`. The server has put its own versions
   of both first on the `PATH` the pane starts with; they answer from the stored content.
   The login profile keeps them when it adds to `PATH` (macOS `path_helper` puts the system
   directories in front and keeps the rest; `export PATH="...:$PATH"`); a profile that sets
   `PATH` from scratch (Debian's `/etc/profile`) would drop them: for zsh, bash and fish the
   server puts them first again after the profile ran (see the pane environment above); for
   other shells `PATH="$TILR_SHIMS:$PATH"` at the end of the profile brings them back.

So the program pastes as it would on a desktop. Verified against the commands Claude Code
2.1.289 runs on Linux; they are spelled out in `crates/tilrd/tests/common/mod.rs`.

Rules:

- The client need not wait for `ClipboardAck` before sending the key: the server handles
  messages of a connection in order.
- The first chunk of an upload empties the clipboard, so a failed upload never leaves an
  older image to be pasted by mistake.
- An upload over the limit is refused with a `ClipboardAck` carrying the reason. The limit
  is in `HelloAck.clipboard_max_bytes`; a client should check before sending and tell the
  user.
- Content expires after `--clipboard-ttl`.
- While the clipboard is empty, the server's `xclip` and `wl-paste` hand over to the real
  tools further down the `PATH`, if any. They also do for anything they do not serve: the
  primary selection, `wl-paste --watch`, `wl-copy --clear`.

Limits of this approach:

- It serves programs that run `xclip` or `wl-paste`. A program that talks to X11 or Wayland
  directly does not see it.
- The clipboard is one per user and machine. Two clients pasting at the same moment
  overwrite each other.
- It serves Linux programs: the shims are on `PATH` on macOS too, but macOS programs read
  the clipboard through other means (`pbpaste`, the pasteboard) and do not run them.

Files live in `$XDG_RUNTIME_DIR/tilr` (or `tilr-<uid>` under the temp directory), readable
by the user only. To use the clipboard from a shell outside tilr, say an SSH session on the
same machine, put the `bin` directory there first on its `PATH`.

### Copying from a pane

The other way works too: what a program in a pane copies reaches the clipboard of the user's
computer, as it does over ssh in a terminal that takes OSC 52, and as in tmux.

1. The program copies: it prints OSC 52 (or OSC 1337 Copy), as vim, neovim, tmux and most
   terminal programs do over ssh; or it runs `xclip -selection clipboard -i [-t TYPE]
   [FILE…]` or `wl-copy [-t TYPE] [TEXT…]`, which are the server's (`wl-copy` too is first
   on the pane's `PATH`).
2. The copy becomes the server's clipboard, the one `xclip -o` / `wl-paste` in any pane
   read, until `--clipboard-ttl`.
3. The server sends it as `ClipboardWrite` (chunks of 256 KB on the bulk lane) to every
   client watching the pane that lists `clipboard-write` in `Hello.features`. `tilr`
   writes text to its terminal as OSC 52, so the user's terminal puts it on their clipboard
   (if it allows programs to: iTerm2, kitty, WezTerm, Ghostty, Alacritty, Windows Terminal
   do, some only once allowed).

Rules:

- A copy nobody takes (no client that takes copies watches the pane) stays only in the
  server's clipboard: it is not handed to a real `xclip` / `wl-copy`, and a client that
  attaches later is not sent it. The shim exits 0 either way.
- A copy over `--clipboard-max` is refused: the shim exits 1 with `too large: …, the
  clipboard takes at most …` and the clipboard keeps what it held; an OSC 52 copy over it
  is dropped, with `clipboard: a copy over the limit was dropped` in the log. The panes
  have the limit in `TILR_CLIPBOARD_MAX` (bytes).
- A newer copy ends one still being sent to a client: a program copying in a loop holds one
  copy per client in the server, the last.
- OSC 52 reads (`?`) are never answered: a program in a pane cannot read the user's
  clipboard. OSC 52 to the primary selection, and `xclip -selection primary`, are not
  passed on.
- `--no-clipboard` turns copying off with the rest: no `wl-copy`, nothing sent.

## Reloading without closing shells

The server picks up a new executable without ending any shell only when explicitly
requested with `SIGUSR1`. The internal `tilrd reload` command signals only this
user's supervisors from its own installation and starts nothing if none is running.
`tilr update` and the installer request reload after atomically
installing the release. Replacing the executable alone leaves the running server alone.
The server checks the executable at its installation path and execs that path; a
replacement between the check and exec is an accepted risk.

The supervisor execs the new executable in place, keeping its pid, the listening sockets
(TCP, and the Unix socket with its lock) and the pane processes, then has every pane process
do the same. Shells, their screens, scrollback and scroll position carry over: a line
soft-wrapped from the history onto the screen stays one line, a cursor waiting to wrap at
the last column still wraps with the next character, the cursor of a program using a
scrolling region or left and right margins with origin mode is on the same cell, and kitty
images keep their places,
those scrolled into the history too. A pane being closed (`ClosePane`, its program not
gone yet) is still killed at its deadline, 2 s after the close, by the successor.

OSC 8 hyperlinks on the screen and in scrollback become plain text after a reload:
libghostty's VT snapshot formatter does not preserve per-cell links. Their text and styles
remain; newly printed hyperlinks work normally.

A process is exec'd with the arguments it was started with, so it looks the same in `ps`
before and after. What it hands to its successor goes into a file in the runtime directory
that the successor finds by name: `server-<pid>.state` for the supervisor (an exec keeps the
pid), `panes/<pane id>.state` for a pane. The successor removes the file as it reads it. The
files are readable by the user only: a pane's holds its screen and the input not yet
delivered. A state file is written whole or not at all (a temporary file next to it, synced
and renamed): on a full disk the reload does not happen, that process carries on as it was,
and nothing half-written is left.

What to know:

- Client connections are dropped by a reload. Clients reconnect and find the panes as they
  were. A file write (`FsWrite`) cut by it fails and leaves the file as it was: each write
  notes its temporary file in `writes/` of the runtime directory, and a server that starts
  (or has just reloaded) removes the temporary files of writes whose process is gone or was
  itself.
- Install a new version by writing it next to the old one and renaming it into place
  (`install`, `mv`). The server reloads from the path it was started from; a version switch
  done by repointing a symlink is not noticed.
- Before reloading, the server asks the new file which versions of the reload state it reads
  (`tilrd state-formats`). If it does not run, reads another version than the server
  writes, or does not say `reload-check 1` (a release from before the check below), the
  server logs `not reloading` and carries on with the code it has. Such an upgrade needs a
  restart. A server that serves its Unix socket does not reload into an executable that
  does not say `unix-socket 1` there either: the socket would be left with nobody serving
  it. A refused reload keeps the socket and its lock served, so `tilr` finds the same
  server.
- Then, before it stops anything, the server has the new file show that it can resume: it
  runs it with the server's own command line and environment and `TILR_RELOAD_CHECK` naming
  a copy of the state it would hand over (`server-<pid>.check.state`). That run parses the
  command line, reads the configuration file and the state and goes through the resume up to
  taking over the sockets and pipes, answers `ok` or `error: <reason>` on its stdout and
  exits; it binds, locks and starts nothing. The server serves its clients meanwhile and
  execs only on `ok` within 10 s; otherwise it logs `not reloading: the new executable could
  not resume: <reason>` and carries on as it was. A new version that rejects the command
  line (`tilr` starts its server with `--socket`), refuses the configuration file or crashes
  resuming is not reloaded into. Each pane process checks its successor the same way
  (`panes/<pane id>.check.state`) and carries on with its code if the check fails. A new
  version that passes its check and then crashes taking over still ends the server and its
  shells.
- `SIGUSR1`, `SIGHUP`, `SIGTERM` and `SIGINT` that arrive during the exec or while the new
  executable starts are held across the exec and handled once it is up: one more reload,
  ignored, a clean shutdown (SIGTERM and SIGINT). The server takes them on one thread of its
  own (blocked on all the others), which blocks them too before the exec, so that no thread of
  the old image takes one; its children start with none of them blocked. A `SIGTERM` or
  `SIGINT` that comes while the reload gets ready for the exec (the panes detaching, the
  tunnel stopping) is not lost either: it is raised again right before the exec and waits
  for the new executable, which shuts down cleanly. If the threads cannot be made to hold
  the signals within 5 seconds, the reload fails (`reload failed: the signals cannot be
  held ...`) and the server carries on.
- An exec that fails leaves the server (or the pane process) as it was, with `SIGPIPE`
  ignored again: a write to a pipe nobody reads any more fails instead of ending it.
- A pane process that cannot take its queued input within 5 seconds is closed rather than
  carried over.

## Protocol

Defined in [`crates/tilr-proto/proto/tilr.proto`](crates/tilr-proto/proto/tilr.proto). A
packet is one binary WebSocket message: a flag byte (0 = plain, 1 = zstd) followed by a
protobuf `ClientMessage` or `ServerMessage`.

Both sides ignore message types and fields they do not know. The protocol version
(`PROTOCOL_VERSION`, now 1) is the contract: a client and a server of a version implement
every message and field of it, and a server refuses every other version, older or newer.
Before the first public release, all protocol changes use version 1 without a bump.
Afterward, bump the version only for a change incompatible with a client or server of the
previous public release. The bump is a separate release step, once per release, in its
release milestone. A change a peer may ignore (a new push, a new field with a harmless
default) needs neither a new version nor a feature. `Hello.client_id`, `KeyInput.code`,
the `bytes` twins of file paths, `Ping`, the bulk window paced by pongs,
`PaneInfo.viewed_by` / `used_at_ms` and close codes 4000-4002 are all part of version 1.
The Tilr app's `server` plugin and tilr-mac need version 1 to talk to this server;
`tilr` and `tilr-driver` are built with it.

`Frame.links` is a table of hyperlink URIs used by its changed rows, deduplicated within
the frame. `Run.link` is the URI's index plus one, or 0 for plain text; runs split when
the link changes. URIs exceeding the VT engine's OSC payload limit are omitted and
their cells remain plain text. With libghostty-vt 0.2.2 (checked 2026-10-24), a
parameter-free OSC 8 retains a 2046-byte URI and drops a 2047-byte URI; OSC parameters
also use that budget. Recheck with
`cargo test -p tilr-screen the_vt_osc_limit_keeps_text_but_drops_long_links --locked`.
The frame reader also has a defensive 8 KiB URI bound.
`tilr attach` draws linked runs with OSC 8 in the local terminal. These fields have
harmless defaults: clients ignoring them still show the text, with protocol version 1
and no feature flag.

`HelloAck.features` lists only what a server can be run without, so a client checks it
before sending that capability's requests, in this order:

| feature | messages | off with |
|---|---|---|
| `clipboard` | `ClipboardPut`; `ClipboardWrite` to clients that list `clipboard-write` | `--no-clipboard` |
| `files` | `FsList`, `FsStat`, `FsRead`, `FsWrite`, `FsMkdir`, `FsRename`, `FsDelete`, `FsWatch`, `FsUnwatch` | `--no-files`, `[files] enabled = false` |
| `host-events` | `HostEvent` | the host-events socket not up |
| `agent` | SSH agent forwarding (`StreamOpen` with kind `ssh-agent`) | `--no-agent`, `agent = false` |
| `exec` | `ExecStart`, `ExecInput`, `ExecSignal`, `ExecCancel` | `--no-exec`, `exec = false` |

`PaneActivity` is an additive server push to every greeted client and needs no
feature negotiation. See [Pane activity](#pane-activity).

`Hello.features` says what the client can serve: `host-events` (it takes `HostEvent`),
`agent` (it serves the SSH agent), `clipboard-write` (it takes `ClipboardWrite`, the copies
programs in its panes make; see [Copying from a pane](#copying-from-a-pane)).

A connection starts with `Hello`, which carries the protocol version of the client and the
[access token](#who-may-connect). A server that speaks another version, or is sent another
message first, answers with an `Error` and closes the connection. A packet that does not
decode gets an `Error` (`bad packet: …`) and the connection stays, before `Hello` as after;
a message of a type the server does not know is ignored. `HelloAck` carries the
version of the server and `update_channel` (`stable` or `beta`, from `server.toml`,
default `stable`), and is the first message the client gets: nothing else is sent to it
before.

The server sends a WebSocket ping every third of `--client-timeout`. Browsers and WebSocket
libraries answer on their own. A client is dropped, and its panes stop counting it as a
viewer, when nothing came from it for the whole timeout (it dropped off the network without
closing the connection, or its way to the server is cut), or when it still sends (something
within the last ping interval) but answered no ping for the whole timeout: it no longer
hears the server, its way from the server cut. What the client sends, `Ping` included, does
not keep a connection alive whose other direction is dead. The log says which (`client went
silent`, `client stopped answering pings`), and the server closes the connection with
WebSocket close code 4002 (`timeout`), or resets it if not even the close gets out.

A client is one instance of an app (a window, a tab, a process), and says so in
`Hello.client_id`: a UUID v4 it makes up when it starts and sends on every connection it
makes. When a `Hello` names the `client_id` of a connection still open, under the same
access token and on the same transport (the Unix socket, or TCP: neither speaks for the
other), the server closes that older connection at once with close code 4000
(`superseded`): it is the same client come back (from another network, after a reload of
its page), and the old connection is what the network it left still holds, a ghost viewer
otherwise until the client timeout. The new connection subscribes again and receives a
full screen. Its stable identity keeps the input delivery sequence: after Hello reports applied counters, the client replays
unacknowledged input in order, and the pane deduplicates it. Admitted input reaches the
pane exactly once while delivery history is retained, even if the old connection revives.
A normal client exit drains acknowledgements and sends `FinalizeInput`, which removes
its history immediately. Disconnected identities expire after one hour without
reconnecting. Pane exec starts with empty delivery history while queued PTY bytes
remain. Replaying unacknowledged input after expiry or reload can therefore duplicate
previously applied bytes (including a partially written frame's prefix). A client does not reconnect a
connection closed as superseded
(`tilr` ends its attach). Another token, or the other transport, with the same id
supersedes nobody; an empty `client_id` neither supersedes
nor is superseded. Numbered input without a stable `client_id` returns
`Error "numbered input requires a stable client_id"` and is not applied.
Numbered input naming no running pane receives `Error "no such pane: <name as sent>"`
followed by `InputAck` for that exact pane string and delivery sequence. This is a
terminal rejection: the client reclaims that pane's queued input through that sequence,
and a lost acknowledgement is returned again on replay. It does not change any pane's
applied counter or add a missing pane to `HelloAck.applied_input`. Unnumbered requests
receive only the error.
Hello probes each pane for applied counters. A pane that does not answer within
five seconds (checked on each supervisor tick) is killed and closed; HelloAck
waits for its link to close and retains counters from responsive panes. Pending
probes are governed by this bound rather than the timeout for missing Hello.
`PaneInfo.viewed_by` lists each viewer's `client_id`, name and address,
and the log has them on connect, hello, supersede and drop.

The server does not hoard frames for a client that cannot keep up. Unsent data the kernel
holds for a client is kept small (`TCP_NOTSENT_LOWAT`, 4 KiB), so what it is behind on
waits in its queue. A client whose queue has held a packet for longer than 250 ms lags: its
frames of a pane are skipped until its queue is written out, then it gets a full frame of
the screen as it is then (the pane is asked for one, within a second even if the pane
prints nothing more; the other viewers get it too), whose
`last_input_seq` carries the client's own latest input if a skipped frame echoed it. A
client that skips gets fewer frames, each up to date, instead of a growing backlog of stale
ones; other clients are not slowed. A client whose queue fills up all the same (1024
packets) is dropped (`client too slow`, close code 4001), and its connection closed at once,
whatever was queued.

Data in quantity does not wait in that queue. A client's queue has two lanes: frames,
answers and acknowledgements in one, and in the other file content (`FsData`), the bytes of
[streams](#ssh-agent-forwarding) (`StreamData`, and the `StreamClose` after them) and the
output of [execs](#running-programs-exec) (`ExecOutput`, and the `ExecExit` after it). The
connection writes from the second only when the first is empty, so a keystroke's echo waits
behind at most the one chunk being written, not behind megabytes of files being read. Nor
behind what the network holds: of the second lane's data, at most 1 MiB is out on the way to
the client and not yet read by it (its receive buffer can hold several MB). The server knows
how far the client has read by WebSocket pings it sends between the chunks, which the
client's WebSocket answers as it reads; a client must answer pings, as browsers and
WebSocket libraries do on their own. Every ping the server sends on the connection, the
liveness ones above included, carries how far it is (`tilr-bulk:` and the position as a
big-endian u64), so a client that answers only the latest of several pings, as RFC 6455
allows, moves the window on all the same; a pong that claims more than was sent counts as
what was sent. The second lane holds at most 4 MiB per client: a file
read, a program sending to a stream, or an exec's program writing output, waits for room, so
a client that takes data slowly holds the read or the program back, while the server's
memory stays flat, and is not dropped for being slow.

A page in a browser cannot see those pings, so for the other direction there is `Ping`,
which every server has (no feature announces it): the server answers `Pong` with the same
nonce at once. A client that
pings regularly and gets no answer in time knows the server is out of reach, and
reconnects.

### Keys

`KeyInput` names a key by what the layout makes of it (`codepoint`, the unshifted character,
and `text`, what it typed) and, in `code`, by the physical key: the name W3C
`KeyboardEvent.code` gives it (`"KeyC"`, `"Digit1"`, `"BracketLeft"`, ...). A client sends
both. In a browser they are `event.key` and `event.code`; on macOS
`NSEvent.charactersIgnoringModifiers` and `NSEvent.keyCode` mapped to the code name
(`kVK_ANSI_C` is `"KeyC"`); on iPad `UIKey.charactersIgnoringModifiers` and `UIKey.keyCode`,
a HID usage (`UIKeyboardHIDUsageKeyboardC` is `"KeyC"`).

The physical key is what Ctrl goes by on a layout that is not Latin, as in other terminals
(xterm, kitty, Ghostty): Ctrl+C on a Russian layout is the key that types "с", and the
program gets `^C` for it. When Ctrl is held and the key's character is not ASCII, a
character with no control code, the server encodes the key as its character on a US layout
(Ctrl+С `^C`, Ctrl+Я `^Z`, Ctrl+Alt+С `ESC ^C`). A character that is ASCII goes by itself
wherever its key is: Ctrl+C on Dvorak (the I key) is `^C`, Ctrl+A on AZERTY (the Q key)
`^A`. Alt without Ctrl sends `ESC` and the character the key produced, Shift included,
never remapped: Alt+ж is `ESC ж`, Alt+Shift+Ф `ESC Ф`, Alt+Shift+1 `ESC !` (with or without
`code`; plain text when the program turned the ESC prefix off, mode 1036). AltGr and macOS
Option composing produce a character, which the client sends without Alt and the program
gets as it is (`€`, `ç`); whether Option acts as Alt is the client's setting (like
Ghostty's `macos-option-as-alt`), and the server takes a key as Alt only when `mods` says
so. Typing without Ctrl or Alt is the text. With the kitty keyboard protocol on, the
physical key is the base layout key it reports. A key without `code` has no physical key
(synthetic input from a script, or a character key of `tilr`, which reads keys from its own
terminal as characters): it is taken as on a US layout, and Ctrl with a character that is
not ASCII sends nothing.

### File names that are not UTF-8

The files protocol (feature `"files"`) carries paths as strings, and a name on Linux can be
any bytes. A name that is not UTF-8 comes with its raw bytes as well (`FsEntry.name_bytes`,
`path_bytes` in `FsEntries`, `FsInfo`, `FsChanged`), its string being for showing only
(U+FFFD for each bad byte); every request has the same `bytes` twin of its path
(`path_bytes`, `from_bytes`, `to_bytes`), which the server goes by when it is not empty. A
client sends back the bytes it was given, so it can read, write, rename and delete such a
file.

### Measuring input latency

The terminal pipeline can be measured end to end without synchronising clocks. A client
names an input message by `ClientMessage.seq` (outside the `oneof`) on `KeyInput`,
`MouseInput`, `Paste`, `RawInput`, `Scroll` or `Resize`: `HelloAck.seq_base + n`, with `n`
counting 1, 2, 3, ... on the connection. `seq_base` is the connection's own (its number in
the high 32 bits; numbers go on across a reload of the server), so a seq says which
connection's input it was, `seq_base <= seq < seq_base + 2^32`, even with several clients on
a pane or after a page reload.

Every `Frame` carries `last_input_seq`: the seq of the input the pane credited last before
it built the frame, from whichever client; 0 until there is one. Key, mouse, paste and raw
input is credited once the pty has taken its last byte, in the order it came; `Scroll` and
`Resize` when applied, which is at once, so they may overtake key input still waiting for
the pty. It is the latest, not the highest: another client's input moves it, down as well.
Every credited input gets a frame. When it changed nothing on the screen (a click the
program ignores, a scroll that moves nothing, input not echoed), that frame has no rows:
`full` false, no `changed`, `styles`, `title` or `images`, `placements_changed` false, and
`cols`, `rows`, `cursor`, `mouse_tracking`, `alt_screen`, the cell size and `viewport_top`
as before, so applying it changes nothing. Inputs credited between two frames show only the
last of them; a client takes a frame carrying a later seq of its own as the echo of the
earlier ones too.

The time from sending input `n` to the first frame carrying its seq (or a later one of the
same connection) is the latency the user sees, measured on the client's clock alone. The
frame that shows the echo usually comes a frame or two later; `FrameTiming.output_bytes`
tells which frames carry output. Input without a seq makes no frame of its own. The fields
are optional: a server that does not know `seq` ignores it and sends `seq_base` 0, a client
that sends none gets 0. A client that ignores `seq_base` and numbers from 1 still gets its
numbers echoed, but cannot tell them from another connection's.

Every frame also carries `timing` (`FrameTiming`), the pane process's own account, in
microseconds:

| Field | What |
|---|---|
| `wait_us` | First pty output since the previous frame to the start of this frame's build: coalescing (frames are at least 8 ms apart) and scheduling. 0 for a frame no output caused. Output that changed nothing on the screen makes no frame and is counted in the next one, however much later |
| `parse_us` | Feeding the output into the screen model, summed |
| `build_us` | Building the frame from the screen model (diffing rows) |
| `encode_us` | Protobuf encoding and zstd compression of the pane's *previous* frame: a frame cannot hold its own |
| `output_bytes`, `reads` | Pty output since the previous frame, and the reads that brought it |
| `input_us` | For a new `last_input_seq`: the pane process receiving the input to the pty write that took its last byte returning (`Scroll`, `Resize`: to it being applied). 0 when no new input. Input still waiting for the pty when the server reloads keeps its time |

The supervisor's part (routing between client and pane) is in [`/metrics`](#metrics).

## Metrics

`GET /metrics` on the server's port answers a plain HTTP request (not a WebSocket) with the
server's counters in the Prometheus text format (`Content-Type: text/plain; version=0.0.4;
charset=utf-8`). A request whose line starts with `GET /metrics` followed by a space or `?`
is taken for one, so `/metrics` and `/metrics?...` are no longer paths a WebSocket can be
opened on.

It asks for the access token as [`/socks`](#socks-proxy) does: `Authorization: Bearer
<token>`, or `?token=<token>`. Without it: 401; from a browser page of a disallowed origin
(`Origin`): 403. A server without a token (`--no-auth`) asks for none, but answers only this
machine: the `Host` header must be loopback (`localhost`, `127.0.0.1` or `[::1]`, with or
without a port) or the address the server listens on (not `0.0.0.0`), and a request with
`Sec-Fetch-Site: cross-site` (a page of another site) is refused; 403 otherwise. A response
not read within 5 s is dropped.

```sh
curl -H "Authorization: Bearer $(tilr token)" http://127.0.0.1:7890/metrics
```

| Metric | Type | What |
|---|---|---|
| `tilrd_info{version,server_id}` | gauge | Always 1 |
| `tilrd_clients` | gauge | Connected clients |
| `tilrd_panes` | gauge | Running panes |
| `tilrd_pane_frames_total{pane}` | counter | Frames the pane produced |
| `tilrd_pane_frame_bytes_total{pane}` | counter | Their bytes as routed, once per frame whatever the number of clients |
| `tilrd_pane_frames_sent_total{pane}` | counter | Frames handed to clients, one per frame and client |
| `tilrd_pane_frames_skipped_total{pane}` | counter | Frames not sent to a client that lagged (it got the whole screen later), one per frame and client |
| `tilrd_client_drops_total{reason}` | counter | Clients the server let go: `superseded` by a newer connection of the same client, `too_slow`, `silent`, `no_pong` (sent, but did not answer pings), `no_hello` (no Hello before the handshake deadline) |
| `tilrd_pane_input_total{pane}` | counter | Input packets (key, mouse, paste, raw, scroll) passed to the pane |
| `tilrd_pane_input_bytes_total{pane}` | counter | Their bytes |
| `tilrd_route_seconds` | histogram | A pane's packet, from the supervisor reading it off the pane process's pipe to the WebSocket write to the client returning (the client's queue included) |
| `tilrd_route_input_seconds` | histogram | A client's input packet, from the supervisor reading it off the WebSocket to the write to the pane process's pipe returning |
| `tilrd_supervisor_loop_lag_seconds` | histogram | How late the supervisor's event loop gets to a once-a-second tick. The tick comes from tokio's timer, which works in whole milliseconds: up to about 1 ms of every observation is the timer's, so the buckets under 1 ms say little |
| `tilrd_process_cpu_seconds_total{role,pid,pane}` | counter | User and system CPU time of the supervisor (`role="supervisor"`, no `pane`) and of each pane process (`role="pane"`) |
| `tilrd_process_memory_bytes{role,pid,pane}` | gauge | Their memory: resident set on Linux, physical footprint on macOS |

The histograms have buckets from 50 µs to 100 ms (0.00005, 0.0001, 0.00025, 0.0005, 0.001,
0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, +Inf). The pane counters start from zero when the
supervisor does, a reload included. CPU and memory are read when scraped. The pane process
(the shell is its child) is what is measured, not the programs in the pane.

**Breaking change:** the following metric families were renamed. Replace the old names in
Prometheus queries and alerts; the server emits only the new names.

| Old | New |
|---|---|
| `tilr_pane_frames_total` | `tilrd_pane_frames_total` |
| `tilr_pane_frame_bytes_total` | `tilrd_pane_frame_bytes_total` |
| `tilr_pane_frames_sent_total` | `tilrd_pane_frames_sent_total` |
| `tilr_pane_frames_skipped_total` | `tilrd_pane_frames_skipped_total` |
| `tilr_pane_input_total` | `tilrd_pane_input_total` |
| `tilr_pane_input_bytes_total` | `tilrd_pane_input_bytes_total` |
| `tilr_client_drops_total` | `tilrd_client_drops_total` |
| `tilr_route_seconds` | `tilrd_route_seconds` |
| `tilr_route_input_seconds` | `tilrd_route_input_seconds` |
| `tilr_supervisor_loop_lag_seconds` | `tilrd_supervisor_loop_lag_seconds` |
| `tilr_process_cpu_seconds_total` | `tilrd_process_cpu_seconds_total` |
| `tilr_process_memory_bytes` | `tilrd_process_memory_bytes` |

## Tests

```sh
cargo nextest run --workspace --features tilrd/failpoints --locked
scripts/linux.sh                  # the same suite in the Linux build container
python3 scripts/test_server_cleanup_stand_selftest.py  # check the stand itself
python3 scripts/test_server_cleanup_stand.py           # signal/timeout cleanup diagnostic
```

The tests in `crates/tilrd/tests/` start the real server and drive it as a client would;
`crates/tilr/tests/` run `tilr` against the `tilrd` built next to it (so they are tests of
the workspace, not of `tilr` alone). [USE_CASES.md](USE_CASES.md) lists the use cases they
cover — connecting, the Unix socket, the client, reconnecting, scrolling, reloading,
updating, the clipboard, files, the SOCKS proxy, the SSH agent, exec, metrics — and what is
not covered yet. They need `/bin/sh`, `ps` and `kill`; no network.

The shared `open_pane` and `open_pane_with` helpers wait for the isolated shell's
first `$ ` prompt before returning, so a first command's PTY echo cannot precede
shell startup and leave its output prefixed by that prompt. Tests that deliberately
exercise input before startup use `pane_before_shell_prompt` and explicitly release
its FIFO; custom-command panes keep their own readiness protocol. Independent
startup and output gates use named `activity_fifo` paths.

Integration test servers use an internal ownership connection
(`TILR_TEST_OWNER_SOCKET`) independent of their command socket. The harness registers
a process exit watcher before acknowledging startup. Closing ownership, including
when the test process receives SIGTERM or SIGKILL, requests normal shutdown of the
server and its panes. Executable reloads reconnect to the same owner.

The normal `cleanup_stand` integration test runs panic, startup panic, signal and
controlled timeout cases against both original and copied executables. To select
cases manually, use `python3 scripts/test_server_cleanup_stand.py --cases panic
panic-startup signal timeout`; add `--copied` to exercise a private executable copy.
The stand checks pane and shell exit events as well as the supervisor.

The cleanup stand owns a temporary directory for each case and removes it, including
notification and runtime sockets, after cleaning its processes. It runs an ignored
test fixture outside the normal suite. It waits for
the server's ready notification before interrupting the test process, checks the
server PID (and `pgrep` on macOS), and removes its own processes even when a check
fails. Success requires an exit event for that exact PID (kqueue on macOS,
pidfd on Linux); an unavailable watcher is an error. SIGTERM to the stand itself
also runs cleanup and exits with status 143. A 30-second deadline bounds
reporting a surviving server. It exits nonzero when an interrupted test leaves its
server running. The pgrep-failure selftest runs only on macOS, matching the stand's
platform check.

The `failpoints` feature compiles in the places where a test can make an update or a reload
go wrong halfway: fail a step, crash or kill the process there (`crates/tilrd/src/fail.rs`).
Tests can also hold a point with `name=pause`: bind `TILR_FAILPOINTS_SOCKET` before
triggering the operation, receive the point name plus newline, and send `r` to release
that hit. `TILR_FAILPOINTS_FILE` supports names learned at runtime. The harness provides
`PauseController::wait` and `PausedPoint::release`; `fail.rs` documents the protocol,
error handling and how to call a blocking point from a Tokio task.
Reconnect controls in `supervisor.rs` are `reconnect-client-accepted` (after the
WebSocket handshake, before registration), `reconnect-unwatched-output:<pane id>`
(a frame received without viewers), `reconnect-subscribe:<pane id>` (before
registration and the full-frame request), `reconnect-full-frame:<pane id>`
(before enqueueing a full frame for each viewer), and
`reconnect-client-lagging:<pane id>` (before the first skipped delta). The latter
four hold supervisor event processing; socket tasks keep running. Configure only
the hits needed and release each before awaiting another supervisor response.
The test client's `pause_reads` acknowledges the pause on return; its handle's
`resume` permits reads again, while sending remains possible throughout.
Without it those tests are left out and the rest runs. A release is built without it.
Tests can also set `TILR_TEST_TIME_FILE` to a file containing Unix microseconds and replace
that file atomically between protocol events. The supervisor reads it for pane timestamps,
and pane processes use it for frame timing. Raw SOCKS sessions use the same clock for their
ten-second negotiation deadline; each publishes a local wake-up port beside the clock file,
named for its client's TCP port. These controls exist only in `failpoints` builds.

With that clock held fixed, `TILR_TEST_FRAME_STAGE_TIME_FILE` can name another file
containing three positive microsecond durations, e.g. `11 23 37` for parse, build and
encode. Each operation advances only the measurement clock, giving exact per-read
parse durations, per-frame build durations, and the previous frame's encode duration.
Atomically replace the stage file between acknowledged frames to change the values.
Scheduling and deadlines continue to use their existing clocks. The
`frame-stage-no-frame=pause` failpoint acknowledges a build that emitted no frame,
so a test can accumulate invisible output across separate reads without a timing race.

The black-box tests in `itests/` ([itests/README.md](itests/README.md),
[itests/USE_CASES.md](itests/USE_CASES.md)) install and run the real binaries in containers:
the installer, the daemon's life, services, sockets, updates, transfers.

CI (`.github/workflows/ci.yml`) runs `cargo fmt --all --check`, clippy with `-D warnings`,
the suite on Linux and macOS, `cargo check` on the MSRV, `cargo deny check` and gitleaks.
`scripts/update-test.sh HOST VERSION` checks an update on a real machine against the real
CDN: it installs an older release there, opens panes and updates them to the latest one.

## Build

Needs Rust 1.99+, [Zig](https://ziglang.org) 0.15.2 and `protoc`, plus network access on the
first build (libghostty-vt fetches the Ghostty source). `scripts/install-tools.sh DIR`
downloads the pinned Zig, protoc and cargo-nextest (the tests run with it) into `DIR` and
checks their SHA-256 sums.

```sh
cargo build --release --locked -p tilrd -p tilr
```

**Shared Ghostty VT development build.** Install `pkg-config` and Python 3, then build once per
machine and export the printed prefix's package directory:

```sh
prefix=$(scripts/build-ghostty-vt.sh)
export PKG_CONFIG_PATH="$prefix/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cargo build --tests --locked
```

The script prints only the prefix to stdout. Its default cache is
`~/.cache/tilr/ghostty-vt/<ghostty-commit>/<identity-hash>/`; override the cache root
with `TILR_GHOSTTY_VT_CACHE`. The identity includes the Ghostty commit, actual Zig
version, OS/architecture, SDK/developer paths and ReleaseFast/baseline settings.
It selects the same default macOS SDK as Cargo, respecting `SDKROOT` and
`DEVELOPER_DIR`. Completed prefixes are reused; failed builds are removed.
Concurrent invocations for the same prefix report the active build; rerun after
it finishes. The lock records the owner's PID. If that process has exited, the
next invocation removes the abandoned lock and temporary build directories and
builds again automatically, including when no PID was written. An advisory lock
serializes recovery; its empty `.guard` file remains in the cache and must not be
removed while a build is running. Incomplete destination prefixes are replaced before
the completed installation is published.

`tilr-screen` enables `libghostty-vt-sys`'s pkg-config feature directly. With this
package discoverable, clean targets link the shared static library without cloning
or building Ghostty. Without a discoverable package, the existing Zig source build
remains available. `GHOSTTY_SOURCE_DIR` bypasses pkg-config, so unset it when using
the shared library. Linux containers retain their source-build path because
`scripts/linux.sh` does not forward the host package path.

The Linux image seeds Zig's package cache with the pinned ImGui, zigimg and uucode
archives from `codeload.github.com`, checking their hashes against Ghostty's package
manifests. This avoids the GitHub redirect response: checked 2026-10-24, its CSP
header alone exceeds Zig 0.15.2's 8 KiB HTTP read buffer. To recheck, run
`curl -sSI https://github.com/ocornut/imgui/archive/refs/tags/v1.92.5-docking.tar.gz`
and compare the response header size with Zig's `lib/std/http/Client.zig`
`read_buffer_size`. When upgrading Ghostty, update the image's package pins too.

When upgrading `libghostty-vt`, keep the direct sys dependency at the matching exact
version. Read that version's `build.rs`, update this script's Ghostty revision and
build flags, and rerun the script. A Zig or SDK change also selects a new prefix;
export its package path again. To rebuild identical inputs, unset `PKG_CONFIG_PATH`,
remove only the previously printed prefix, and rerun the script. Old prefixes may
be removed when no builds use them.

**macOS (checked 2026-10-19).** On this Mac, Zig 0.15.2 and the default macOS
27.0 SDK make `libghostty-vt-sys` 0.2.2 fail while building libc++ (`INFINITY`
undeclared). A clean build succeeds with the Command Line Tools MacOSX15.4.sdk.
The tracked `.cargo/config.toml` selects that SDK in every worktree, including
for direct `cargo build` commands. Install Command Line Tools containing
MacOSX15.4.sdk from Apple Developer Downloads if it is absent; `scripts/install-tools.sh
DIR zig` checks for it before downloading Zig. Set `DEVELOPER_DIR` and `SDKROOT`
to an Xcode installation with macOS 15.4 SDK if Command Line Tools does not
contain it. The macOS CI and release runners use Xcode 16.3 for this reason
([runner image inventory](https://github.com/actions/runner-images/blob/main/images/macos/macos-15-arm64-Readme.md)). Recheck the installed SDK with
`DEVELOPER_DIR=/Library/Developer/CommandLineTools xcrun --sdk macosx15.4
--show-sdk-path` and the default SDK with `xcrun --show-sdk-version`. After
upgrading to a Zig release that supports the current SDK, rerun a clean
`CARGO_TARGET_DIR=$(mktemp -d) cargo build --workspace --features
tilrd/failpoints` without the SDK pin in a separate worktree. Then remove the
pin from `.cargo/config.toml`, the installer preflight, and this compatibility
note (TILR-165).

**Linux, from another OS.** `scripts/linux.sh` runs a command in a container
(`scripts/linux.Dockerfile`: Debian 12, the pinned Zig) with the source mounted, as a
non-root user `runner` like the CI runner (`TILR_LINUX_USER=root` for root):

```sh
scripts/linux.sh                  # the test suite
scripts/linux-build.sh            # release binaries into dist/<platform>/
TILR_LINUX_PLATFORM=linux/amd64 scripts/linux-build.sh
```

Each physical worktree has its own persistent target cache, named
`tilr-linux-target-<path hash><platform suffix>`. The hash is Git's hash-object of
the physical absolute checkout path plus a newline; the platform suffix replaces
each non-lowercase-alphanumeric character with `-` (e.g. `linux-amd64`). Separate
worktrees can build concurrently, even with identical directory names. Serialize
runs within one worktree. The cargo registry remains shared per platform.
`linux-build.sh` builds and copies from the same worktree's target.
The container builds the dev profile with line-tables-only debuginfo, since the
host's `~/.cargo/config.toml` is not visible there; export `CARGO_PROFILE_DEV_DEBUG`
to override it (e.g. `CARGO_PROFILE_DEV_DEBUG=full scripts/linux.sh ...` for gdb with locals).

To clear a worktree's target, stop its containers, then run from its root:

```sh
docker volume rm "tilr-linux-target-$(pwd -P | git hash-object --stdin)"
# For TILR_LINUX_PLATFORM=linux/amd64, append linux-amd64 to that volume name.
```

The old shared `tilr-linux-target` (or its platform-suffixed variant) is no longer
used and can also be removed once all older runs have stopped.

## Releases

`main` is the only integration branch: task branches are squash-merged into it through pull
requests, and nothing is released from it. On the owner's word a release branch `release/X.Y`
is cut from a commit of `main`; a fix for a release lands in `main` first and is then
cherry-picked into `release/X.Y`. Nothing is merged from a release branch back into `main`.

PRs into `release/**` run `release-upstream`: every proposed commit must be in main
or match an upstream patch (a cherry-pick may have a different SHA). A new patch,
an unmatched merge or an unmatched empty commit fails the check with an instruction
to land it in main first. A cherry-pick trailer alone does not authorize a patch.
All authors follow this rule. Required checks for release branches must include
`release-upstream`.

Stable releases start with an owner-created `vX.Y.Z` tag from `bin/release`.
`.github/workflows/release.yml` calls the pinned shared workflow on release branch
pushes and stable tag pushes, excluding RC tags to prevent recursion. The tag must
point into the history of `release/X.Y`; missing branches or unrelated commits fail
before any release is created. No conventional commit format, release pull request,
version bump commit or generated CHANGELOG.md is required. The numeric tag version
must be at least `[workspace.package] version` in Cargo.toml; Cargo files stay unchanged,
and `TILR_BUILD_VERSION` stamps the tag version into both binaries. GitHub generates
stable release notes from earlier stable tags. A stable tag creates the GitHub release and then:

- builds `tilrd` and `tilr` for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` and attaches
  `tilr-vX.Y.Z-TARGET.tar.gz` + `.sha256` to the release (`scripts/package.sh`)
  (`aarch64-unknown-linux-gnu` only while the repository variable `RELEASE_LINUX_ARM64` is
  `true`: its runner is free only for public repositories);
- publishes the Linux and signed, notarized macOS archives to the CDN, the R2 bucket `tilr-cdn` behind
  `https://cdn.tilr.dev`, with `scripts/release.sh --upload-only`.

Releases open no pull request in `tilr-dev/desktop-mac`. Tilr bundles the latest
stable tilrd when it is built, only as an offline starting point, and keeps the
installed tilrd and tilr on the latest release by running `tilrd update` at launch
and from its Update tilrd button.

Every push to `release/X.Y` whose commit has no stable tag on that line builds a release candidate: the pushed commit is tagged
`vX.Y.Z-rc.N` and a GitHub prerelease of that tag gets the same archives and checksums
(`Z` is 0 until `vX.Y.0` is released, then the next patch; `N` counts the candidates of that
version). The binaries say `X.Y.Z-rc.N-<commit>`. Candidates are published to the CDN beta channel: archives and checksums go under
`releases/<version>/`, followed by `releases/<platform>/beta.manifest.json` and
`releases/<platform>/beta`. They never change
`latest` or `install.sh`. To install a candidate on a test machine,
set `update_channel = "beta"` in `~/.config/tilr/server.toml` (or the configured
server.toml path), then run `tilrd update`. With no file or key, or with
`update_channel = "stable"`, it reads `latest`. Invalid values fail before CDN access.
The command no longer accepts `--channel`. There is no channel environment variable.
`tilr update` runs `tilrd update` on the server and follows its configured update channel;
the installer and `fetch-release` continue to use stable.
Missing beta fails without falling back to stable. Stable
publication leaves beta unchanged: a stable release follows beta testing.

Before the first signed release, provision an Ed25519 key pair outside the checkout:

```sh
umask 077
openssl genpkey -algorithm ED25519 -out update-signing.pem
openssl pkey -in update-signing.pem -pubout -outform DER -out update-public.der
# The last 32 bytes of the Ed25519 SubjectPublicKeyInfo are the raw public key.
tail -c 32 update-public.der | od -An -v -tx1 | tr -d ' \n'
```

Set repository variable `TILR_UPDATE_PUBLIC_KEY` to those 64 hex characters before building
release binaries. Put the complete PEM contents in secret `TILR_UPDATE_SIGNING_KEY` of
GitHub environment `cdn`; retain a secure backup and remove the local private-key file.
Only the CDN publication step receives that secret. Release builds reject a missing key
and the debug fixture key. Public-key changes require a new binary; runtime overrides
are ignored. Before changing keys, distribute a build that trusts the replacement key.

Manual builds also need `TILR_UPDATE_PUBLIC_KEY` exported; manual publication additionally
needs `TILR_UPDATE_SIGNING_KEY` containing the PEM. The signer requires OpenSSL with
Ed25519 support (OpenSSL 3.0 or newer); `OPENSSL` can name its executable. On macOS it
prefers Homebrew OpenSSL to the system LibreSSL. It checks the private/public pair and
prepares every selected manifest before uploading anything. Each archive and checksum
is uploaded before its signed channel manifest and legacy pointer. Manifest envelopes
are JSON with a `payload` string and hex `signature`; Ed25519 covers the exact UTF-8 payload
bytes. Payload fields are `version`, `channel` (`stable` or `beta`), `platform`, `archive`, and `sha256`.
The updater requires the signed channel to match the configured `update_channel`; intentional rollbacks and
replay of previously signed releases within the same channel remain possible.

Old clients and the installer continue to read the unsigned pointer and `.sha256`; removal
of that legacy layout follows a signed release separately. During publication a request
may observe mismatched metadata and pointer; it fails closed and can be run again.

The CDN layout is what the installer and `tilrd update` read:

```text
releases/<platform>/latest                                 the stable version for the platform
releases/<platform>/beta                                   the candidate version for the platform
releases/<platform>/latest.manifest.json                    signed stable metadata
releases/<platform>/beta.manifest.json                      signed candidate metadata
releases/<version>/tilr-<version>-<platform>.tar.gz        tilrd and tilr
releases/<version>/tilr-<version>-<platform>.tar.gz.sha256
```

A CDN version is the package version plus the commit, e.g. `0.1.0-26449dd`; a platform is
`linux-aarch64`, `linux-x86_64`, `macos-aarch64` or `macos-x86_64`. Each platform has
its own channel pointer, uploaded after its archive and checksum. By
hand, from a clean checkout, `scripts/release.sh --publish linux-aarch64 linux-x86_64`
builds in the Linux container and uploads with AWS CLI. Install AWS CLI and OpenSSL
first. Use `--publish --channel beta` or `--upload-only --channel beta` for candidate
publication; both use the same local `dist/cdn` staging layout (with `latest`).
Infisical `/cloudflare` in `prod` holds `CF_ACCOUNT_ID` and the bucket-scoped
`R2_CDN_TOKEN`; it does not hold the derived AWS keys. `AWS_ACCESS_KEY_ID` is that
API token's **ID** (the `id` returned when creating the Cloudflare Account API token),
and `AWS_SECRET_ACCESS_KEY` is the lowercase hexadecimal SHA-256 of the token value,
with no trailing newline included in the hash. See [Cloudflare's credential derivation](https://developers.cloudflare.com/r2/api/tokens/#get-s3-api-credentials-from-an-api-token).

Replace `YOUR_R2_CDN_TOKEN_ID` below with the ID belonging to `R2_CDN_TOKEN` (after
rotation, use the new token's ID). Run from the checkout with your Infisical project
selected and access to `prod`:

```sh
infisical run --env prod --path /cloudflare -- sh -eu <<'SH'
export CLOUDFLARE_ACCOUNT_ID="$CF_ACCOUNT_ID"
export AWS_ACCESS_KEY_ID='YOUR_R2_CDN_TOKEN_ID'
AWS_SECRET_ACCESS_KEY=$(printf '%s' "$R2_CDN_TOKEN" | openssl dgst -sha256 -r | cut -d ' ' -f 1)
export AWS_SECRET_ACCESS_KEY
scripts/release.sh --publish linux-aarch64 linux-x86_64
SH
```

The keys are passed through the process environment; keep shell tracing off and do
not save their values in the checkout. `--upload-only` uses the same credentials.

Verified on 2026-10-25 with AWS CLI 2.38.5: `aws s3 cp` using the keys derived from
`R2_CDN_TOKEN`, the R2 endpoint and region `auto` successfully uploaded a 20 MB file
in three multipart parts with the AWS CLI v2 checksum defaults (`when_supported`,
CRC64NVME). Neither `AWS_REQUEST_CHECKSUM_CALCULATION` nor
`AWS_RESPONSE_CHECKSUM_VALIDATION` needs to be set for this upload.

In CI, the `cdn` environment supplies both AWS secrets and the
`CLOUDFLARE_ACCOUNT_ID` variable. The release version comes from the tag; the Cargo
workspace version is development metadata. Releases do not modify Cargo.toml or
Cargo.lock, and CHANGELOG.md is no longer generated. Release notes live in GitHub
Releases.

### macOS CDN releases

The release workflow compiles `macos-aarch64` and `macos-x86_64` without
credentials, signs `tilrd` and `tilr` with Developer ID on a separate runner, and requires
accepted Apple notarization before packaging either GitHub or CDN artifacts.
The CDN uses `releases/macos-<arch>/latest` and
`releases/<version>/tilr-<version>-macos-<arch>.tar.gz` with its `.sha256`.
The same signed executables go into both distributions. `tilrd update` uses
this layout on either Mac architecture; first installation comes from the app.

Dan must provision these GitHub secrets in the `apple-signing` environment (restricted to
`main`) before the first macOS release:

- `APPLE_CERTIFICATE_P12_BASE64`: base64-encoded Developer ID Application
  certificate **with its private key**, exported as PKCS#12.
- `APPLE_CERTIFICATE_PASSWORD`: password for that PKCS#12 export.
- `APPLE_SIGN_IDENTITY`: full `Developer ID Application: … (TEAMID)` identity,
  using the same signer as the bundled daemon.
- `APPLE_API_KEY_P8_BASE64`: base64-encoded App Store Connect API key (`.p8`) for
  notarization: App Store Connect → Users and Access → Integrations → Team Keys,
  role Developer. Not tied to a personal Apple ID and revoked on its own.
- `APPLE_API_KEY_ID`: that key's Key ID.
- `APPLE_API_ISSUER_ID`: the team's Issuer ID (above the key list).

The build job has no environment or Apple secrets and uploads unsigned binaries
with SHA-256 checksums. A fresh `sign-macos` runner verifies those checksums,
imports a non-exportable private key into a temporary keychain, stores the
notarization API key there (its `.p8` file is deleted right away), signs and
notarizes without checkout or build commands, then uploads signed artifacts.
The keychain is removed even on failure. Signing metadata is never exported
through `GITHUB_ENV`. A separate `package-macos` job uses only the signed artifacts
for GitHub and CDN packaging. Bucket credentials remain in the final CDN job.
No secret values belong in the repository or command output.

For a manual build on macOS, install the two Rust Apple targets and configure
`TILR_SIGN_IDENTITY` and `TILR_NOTARY_PROFILE` (an existing notarytool keychain
profile), then run `scripts/release.sh macos-aarch64 macos-x86_64` from a clean
commit. `TILR_SIGN_KEYCHAIN` optionally names the keychain containing both the
signing identity and notary profile. `--publish` also requires the CDN credentials;
`--upload-only macos-aarch64 macos-x86_64` publishes completed layouts without
Apple credentials or rebuilding.


## License

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE).
