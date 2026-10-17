# tilr

A terminal server. It runs shells in panes on a machine and serves them to
clients over a WebSocket: a client gets the screen as a grid of cells and sends
keys back. Panes keep running when clients disconnect.

`tilrd` is the server, a service: `tilr` starts it, or a service manager, or
the Tilr app; nobody runs it by hand. `tilr` is the command a user types: the
[terminal client](#the-client-tilr) (tmux-style) and every other command
(`tilr token`, `tilr update`, `tilr cloud login`, `tilr ps`).

Status: early (see [TODO.md](TODO.md)).

## Layout

| Crate | What it is |
|---|---|
| `tilr-proto` | The wire protocol: `proto/tilr.proto` and packet framing |
| `tilr-screen` | Terminal screen model (libghostty-vt) that produces frames |
| `tilrd` | The server binary |
| `tilr` | The client: attaches to panes from a terminal, and the user's commands |
| `tilr-driver` | A scripted client for the black-box tests in `itests/` |

One supervisor process accepts clients and routes packets. Each pane is its
own process, a child of the supervisor, which owns the pty, the shell and the
screen model.

A pane opened without a command runs the user's login shell (the password
database's, else `$SHELL`, else `/bin/sh`) as a login shell, `argv[0]` = `-zsh`
and the like, so it reads the user's profile whatever environment the server
was started in (a launchd agent gets the bare system `PATH`). The pane gets
`SHELL`, `HOME`, `USER`, `LOGNAME`, `TERM=xterm-256color`, `COLORTERM`,
`TILR_PANE`; launchd's `XPC_SERVICE_NAME` / `XPC_FLAGS` are dropped. A pane
with a command runs it as given, without a shell.

## Installing and updating

**macOS application distribution.** The Mac app bundles a pinned bootstrap
`tilrd`, without the JavaScript `tilr` CLI. It installs a user LaunchAgent,
then updates the managed server through its own signed server feed, separately
from the application's Sparkle updates. A newer compatible server survives app
upgrades with older bundled copies. Use **Tilr > Check for Server Updates** for
this managed copy; separate installations retain their own updater.
See [Mac release and lifecycle details](../apps/mac/PRODUCTION.md).

Releases are built for Linux on aarch64 (glibc 2.36 or newer): the server
`tilrd-<platform>` and its client `tilr-<platform>`, each with its
`.sha256`. The installer (`curl -fsSL https://tilr.sh | sh`) puts both in
`~/.local/bin`; by hand:

```sh
version=$(curl -fsSL https://cdn.tilr.dev/releases/linux-aarch64/latest)
for name in tilrd tilr; do
  curl -fsSLo ~/.local/bin/$name "https://cdn.tilr.dev/releases/$version/$name-linux-aarch64"
  chmod +x ~/.local/bin/$name
done
tilr
```

To update, in any shell on that machine:

```sh
tilr update
```

`tilr update` has the running server do it (it runs `tilrd update` as a child
of the server, in its environment), then reconnects and prints the version the
server reloaded into. It asks the CDN which version is ready for its platform, downloads it, checks
it against its checksum and that it runs, and puts it in place of the
executable. A running server then [reloads itself](#reloading-without-closing-shells)
and keeps every shell. `--cdn URL` (or `TILR_CDN`) names another place to
fetch releases from.

The server was called `tilr-server` before (`tilrd` is the server, `tilr` the
client that talks to it). Releases are published under both names, the same
file as `tilrd-<platform>` and `tilr-server-<platform>`: `tilr-server update`
on a server from before the rename finds the new release under the name it
asks for and puts it in place of its own executable, which keeps its file name
(and whatever starts it, a systemd unit, say) and is `tilrd` from then on.
`tilrd update` asks for `tilrd-<platform>`, and for `tilr-server-<platform>`
when a release has only that name (rolling back past the rename).

`tilr version` prints what builds the client and the running server are: the
package version, the commit and the platform, e.g. `0.1.0-26449dd
(linux-aarch64)` (`tilr --version`, `tilrd --version`: one each).

`scripts/release.sh --publish` builds a release and uploads it; the layout of
releases on the CDN is described there. `scripts/update-test.sh HOST VERSION`
checks an update on a real machine: it installs an older release there, opens
panes and updates them to the latest one.

## Building

Needs Rust 1.88+, [Zig](https://ziglang.org) 0.15.2 and `protoc`, plus network
access on the first build (libghostty-vt fetches the Ghostty source).

```sh
cargo build --release
```

**macOS.** Zig 0.15.2 cannot build against the macOS 27 SDK. Point the build at
the Command Line Tools and an older SDK in `.cargo/config.toml` (the file is
not tracked):

```toml
[env]
DEVELOPER_DIR = "/Library/Developer/CommandLineTools"
SDKROOT = "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk"
```

**Linux, from another OS.** `scripts/linux.sh` runs a command in a container
with the toolchain installed and the source mounted:

```sh
scripts/linux.sh                  # the test suite
scripts/linux-build.sh            # release binary into dist/
TILR_LINUX_PLATFORM=linux/amd64 scripts/linux-build.sh
```

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
tilr edit FILE | open URL   # inside a pane: ask the clients watching it to open it
```

**The tmux model.** `tilr` talks to the server of this user on this machine
over a Unix socket. With no server running it starts one: `tilrd` next to the
`tilr` executable, else on `PATH`, else `$TILRD`; detached (a grandchild in a
session of its own, no controlling terminal, stdin from `/dev/null`, its log
in `tilrd.log` next to the socket). Two `tilr` started at once on an empty
machine start one server: the first to take the socket's lock starts it and
hands it the lock (`--lock-fd`), the other waits for its socket. The server
outlives the terminal: a hung-up SSH session takes `tilr` with it, not the
server or its shells. `tilr ls`, `kill` and `attach PANE` start nothing.

**Attached.** The terminal is in raw mode on the alternate screen and shows
the pane, sized to the terminal (a resize follows). **Ctrl+] is the prefix**:
Ctrl+] d detaches, Ctrl+] Ctrl+] sends Ctrl+] itself; another key after it is
sent as it is. Keys go as `KeyInput`, with `code` (the physical key) for the
keys a terminal names (Enter, Tab, the arrows, F1…F12); a character key comes
from the terminal as the character the layout made, Ctrl folded in (Ctrl+С on
a Russian layout arrives as ^C), so its `code` stays empty. Pastes go as
`Paste` (bracketed), the mouse only while the program in the pane asks for it
(else the wheel scrolls the history). The pane used last is the one with the
latest `PaneInfo.used_at_ms` (a subscribe, or input).

**Disconnects.** The client has one `Hello.client_id` (a UUID) for as long as
it runs. A connection that drops (a reload of the server, the network) is
made again with it at once, then every 0.1 s doubling to 2 s; the bottom row
says `disconnected …, reconnecting…` meanwhile, and what is typed then is
dropped, not sent later. Over TCP it pings every 5 s and takes 15 s of silence
as a dropped connection. A pane gone after the reconnect, or a local server
that stopped (socket gone, lock free), ends the attach (`[tilrd stopped]`).

**Which server.** `--socket PATH` / `TILR_SOCKET`, or `--connect ws://HOST:PORT`
/ `TILR_CONNECT` with `--token` / `TILR_TOKEN` for a server's TCP port (not
started from here; without a token, the one of this machine's server). Else
`~/.config/tilr/client.toml` (`$XDG_CONFIG_HOME`, or `TILR_CONFIG_DIR`):

```toml
connect = "ws://10.0.0.5:7890"   # or: socket = "/path/tilrd.sock"
token = "…"
```

**Commands run in the server.** `tilr token`, `tilr update [--cdn URL]` and
`tilr cloud login|status|logout` work on what the server owns (its token, its
executable, its link to Cloud): `tilr` asks the running server, starting it if
needed, to run its own executable with those arguments (`ExecStart` with
`server_command`, taken on the Unix socket only, with exec on or off) and
shows the output as it comes; Ctrl+C cancels. There is no offline mode: a
server is started for them. Over `--connect` they are refused: run `tilr` on
that machine.

**In a pane.** The server puts its own `tilr` first on the `PATH` of every
pane (a link to `tilrd`, see [Pasting images](#pasting-images-the-shared-clipboard)):
it answers `tilr edit` and `tilr open` (to the clients watching the pane, over
`$TILR_SOCK`) and hands every other command to the next `tilr` on `PATH`, this
client. The client answers `edit` and `open` the same way, for a pane whose
profile reset `PATH`. Panes get `TILR_SOCKET`, so `tilr ls` in a pane lists its
own server's panes; `tilr attach` refuses the pane it runs in.

## Running

`tilrd` is a service; `tilr` starts it as `tilrd --socket PATH --lock-fd N`.
Run by hand or by a service manager it stays in the foreground and logs to
stderr:

```sh
tilrd                                 # the Unix socket and TCP 127.0.0.1:7890
tilrd --listen 0.0.0.0:7890           # TCP on every address (the token is still asked)
tilrd --no-listen                     # the Unix socket only
```

**TCP.** On by default, on loopback: `127.0.0.1:7890` (`--listen ADDR` or
`TILR_LISTEN`), where the Tilr app, an SSH tunnel (`ssh -L
7890:127.0.0.1:7890`) and the [Cloud tunnel](#tilr-cloud) reach the server
with the access token. `--listen [::]:7890` is both IPv4 and IPv6 where the
system allows dual stack. `--no-listen` turns TCP off. A port that is taken
stops the server with an error that says so (exit status 1), so a service
manager shows it, except in a server `tilr` started: that one warns in its
log and serves the Unix socket alone, without a tunnel, rather than another
port nobody would know of. A Tilr app's server or another user's may hold
7890; `tilr` on the machine works either way. A reload keeps what the server
had: its TCP socket, or none. A connection on the port that has not finished
its WebSocket handshake (or SOCKS or metrics request) within 10 s is closed,
so connections that say nothing hold no descriptor longer; there is no limit
on their number otherwise.

**Sockets.** The Unix socket is always on unless `--no-socket` (not with
`--no-listen`). The socket is `--socket PATH`, else
`$TILR_SOCKET`, else `tilrd.sock` in the runtime directory: `$TILR_RUNTIME_DIR`,
else `$XDG_RUNTIME_DIR/tilr`, else `tilr-<uid>` in the temporary directory
(`$TMPDIR` on macOS, `/tmp` on Linux). The directory is made 0700 (and must be
the user's), the socket 0600, and the server checks each connection's peer
uid (`SO_PEERCRED`, `getpeereid`) is its own: on the socket no access token is
asked. A path longer than a socket can be bound at (107 bytes on Linux, 103 on
macOS) is an error that says so. One server per socket: it holds
`tilrd.sock.lock` (`flock`, with its pid in it) for as long as it runs, a
reload included. What is found at the path when it starts:

- a socket nobody accepts on and whose lock is free (a server killed with
  SIGKILL left it): removed and replaced;
- a socket some other program listens on, a socket of another user, a file
  that is not a socket: an error, nothing removed.

A reload keeps the socket and the lock (their descriptors go to the successor
as the TCP socket's does). Stopping removes the socket and the lock file.

**The Tilr app is unaffected.** It runs `tilrd --listen 127.0.0.1:7890 --config …`
and uses its TCP port and `tilrd token`, `ps --url`, `state-formats` and
`--version`, which stay (hidden from `--help`). With TCP on and no
`--socket`, a socket that cannot be taken (another tilrd, `tilr`'s, serves it;
the path is too long) is only a warning: that server serves TCP alone. If the
app's server took the socket first, `tilr` attaches to it. A server `tilr`
started first holds 7890 in turn (same user, same token): the app's own
server then cannot listen there.

| Flag | Default | Meaning |
|---|---|---|
| `--listen ADDR` | `127.0.0.1:7890` (`TILR_LISTEN`) | The TCP address of the WebSocket, e.g. `0.0.0.0:7890`, `[::]:7890` |
| `--no-listen` | | No TCP port: the Unix socket only |
| `--socket PATH` | see above | The Unix socket; must work when named |
| `--no-socket` | | No Unix socket (not with `--no-listen`) |
| `--allow-origin ORIGIN` | the configuration file's list, else `tilr://app` (the Tilr app's page), `tilr://tilr` (its terminal app) and the web client's | Browser origins allowed to connect; repeatable |
| `--config PATH` | `~/.config/tilr/server.toml` (`$XDG_CONFIG_HOME/tilr/server.toml`), if there | The [configuration file](#the-configuration-file) |
| `--no-auth` | | Do not ask clients for the [access token](#who-may-connect) |
| `--cloudflared PATH` | `cloudflared` | The `cloudflared` to run for a [tunnel](#tilr-cloud) |
| `--no-clipboard` | | Turn the shared clipboard off |
| `--clipboard-max SIZE` | `20MB` | Largest clipboard upload (`512K`, `20MB`, `1G`) |
| `--clipboard-ttl SECS` | `300` | How long uploaded content stays; `0` = until replaced |
| `--client-timeout SECS` | `30` | Drop a client that stays silent, or does not answer the server's pings, this long; `0` = never |
| `--no-auto-reload` | | Do not reload when the executable changes |
| `--socks` | the configuration file's `socks`, else off | Serve the [SOCKS proxy](#socks-proxy) on the same port |
| `--no-agent` | | Turn [SSH agent forwarding](#ssh-agent-forwarding) off (also `agent = false`) |
| `--no-exec` | | Do not let clients [run programs](#running-programs-exec) outside a pane (also `exec = false`) |

Signals: `SIGTERM` and `SIGINT` stop the server and end its shells (`tilr
kill-server` sends SIGTERM and waits for the server and its pane processes to
be gone); `SIGUSR1` [reloads](#reloading-without-closing-shells) it. `SIGHUP`
is ignored: a server started from a terminal (`tilrd &` in an SSH session)
outlives it, and so do its panes, each in a session of its own. Its log then
goes nowhere; redirect stderr to keep it.

## Starting at boot

Optional, and only when asked: nothing starts tilrd at boot but this.

```sh
tilr service install      # start tilrd at boot, and again if it fails
tilr service uninstall    # remove what install added
```

- **Linux with systemd and lingering**: a user unit,
  `~/.config/systemd/user/tilr.service` (`$XDG_CONFIG_HOME`), the name the
  installer uses: `Type=notify` (tilrd says `READY=1` on `$NOTIFY_SOCKET`
  once it listens), `ExecStart=/bin/sh -lc 'exec "…/tilrd"'` (a login shell,
  so the panes get the environment of `~/.profile`, as over SSH),
  `Restart=on-failure`; `systemctl --user enable` (with `--now` unless a tilrd
  runs). Lingering (the user's manager runs from boot, not only while logged
  in) is turned on with `loginctl enable-linger` if it is off; the unit notes
  that, and uninstall turns it off again.
- **No systemd user manager, or lingering refused**: a line in the user's
  crontab, `@reboot /bin/sh -lc '… exec "…/tilrd" --socket … >>…/tilrd.log 2>&1' # tilr service`,
  and tilrd is started now if none runs. The output says which was chosen.
  Neither systemd nor `crontab`: an error, nothing written.
- **macOS**: a launchd agent, `~/Library/LaunchAgents/sh.tilr.tilrd.plist`
  (`RunAtLoad`, `KeepAlive` on failure, the socket named, the log in
  `~/Library/Logs/tilrd.log`), loaded with `launchctl bootstrap gui/<uid>`.

A tilrd that already runs (one `tilr` started) is left alone: the service
takes over from the next boot. Uninstall removes the unit, the crontab line
(the whole crontab if install made it) or the agent, and nothing it did not
write; it stops nothing, so the running server and its shells go on until
`tilr kill-server`.

## Who may connect

On the Unix socket: the user, as above, without a token. On the TCP port a
client must send the server's access token in `Hello.token`. The server
makes the token the first time it runs and keeps it in
`$XDG_STATE_HOME/tilr/token` (`~/.local/state/tilr`), readable by the user
only. To see it, on that machine:

```sh
tilr token
```

A client without the token, or with another, gets an `Error` and the
connection is closed.

`--no-auth` turns the check off: then anyone who can reach the port gets a
shell. A page in a browser must, with or without the token, come from an
allowed origin: `--allow-origin`, else the configuration file's list, else
the built-in one.

## The configuration file

What a machine's operator sets once lives in `~/.config/tilr/server.toml`
(`$XDG_CONFIG_HOME/tilr/server.toml`, or `--config PATH` / `TILR_CONFIG`),
read when the server starts (a reload starts it again, so a change takes
effect on the next `tilr update` or SIGUSR1). A missing file is an
empty configuration; one that does not parse, or names a key the server does
not know, stops it from starting.

```toml
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
```

## tilr Cloud

Optional. Cloud lists the machines of a user, says which are online, and gives
each a tunnel: a name on the internet that leads to the server's TCP port
(a server on its Unix socket alone, `--no-listen`, gets no tunnel), so a phone can
reach a machine that accepts no incoming connections. What goes through the
tunnel does not pass through Cloud.

```sh
tilr cloud login      # prints a link and a code to confirm in a browser
tilr cloud status
tilr cloud logout     # Cloud forgets the machine and removes its tunnel
```

A linked server keeps one outgoing connection to Cloud. When Cloud tells it
to, it runs `cloudflared` (which must be installed) with a configuration it
writes itself: the tunnel leads to the server's own port and nowhere else.
A running server notices a login or a logout within a few seconds.

What Cloud can and cannot do:

- It can open and close the tunnel, and it sees whether the server is online,
  its version and platform.
- It cannot reach a pane: nothing it sends is passed on to one, and it never
  sees the access token. A client that comes through the tunnel needs the
  token like any other.
- A server started with `--no-auth` takes no tunnel.
- The tunnel is in Cloud's Cloudflare account, and TLS ends at Cloudflare:
  whoever controls that account can read what goes through. End-to-end
  encryption between client and server is not there yet.

The link (`cloud.json`, next to the token) holds a token of the machine's own
for Cloud; it is not the access token. `--url` or `TILR_CLOUD` names another
Cloud than `https://cloud.tilr.dev`.

## SOCKS proxy

Off unless the server runs with `--socks` (or `socks = true` in the
configuration file). Then the server is also a SOCKS5 proxy (RFC 1928) on its
own port, so a browser on another machine opens what listens there, a dev
server on `127.0.0.1:3000` say. CONNECT only; BIND and UDP ASSOCIATE get
"command not supported".

Names are resolved on the server ("remote DNS"). `localhost` and every name
under it (`pb.localhost`, `anything.localhost`, RFC 6761) never go to DNS:
they reach services bound on this machine's loopback, 127.0.0.1 first, then
`[::1]`. A browser connects to `localhost` and `127.0.0.1` itself rather than
through a proxy, so a page asks for `pb.localhost:3000` instead.

Two ways in:

- **`GET /socks`, a WebSocket**: the way through a [tunnel](#tilr-cloud), which
  carries HTTP only. The handshake shows the access token: `Authorization:
  Bearer <token>`, or `?token=<token>` for a page, which cannot set headers on
  a WebSocket. A browser page must come from an allowed origin, as for the main
  protocol. Without the token: 401; disallowed origin: 403; proxy off: 404.
  After that, the client's binary messages are the SOCKS bytes it sends and
  the server's are the ones it gets back, any sizes; SOCKS itself asks for no
  authentication (a username and password, if offered, are taken unread). A
  text message closes the WebSocket with 1003. A WebSocket cannot be half
  closed: once the target is done sending, the server closes it with 1000.
  The server pings it like the main protocol's clients (`--client-timeout`).
- **Raw SOCKS5** on the same port, for `ssh -L 1080:127.0.0.1:7890` and the
  like: a connection whose first byte is 5 is a SOCKS client, which must use
  username/password authentication (RFC 1929) with the access token as the
  password, any username. A server started with `--no-auth` asks for none.

```sh
ssh -L 1080:127.0.0.1:7890 my-server
curl --socks5-hostname "x:$(ssh my-server tilr token)@127.0.0.1:1080" http://pb.localhost:3000
```

The token gives what it gives through the main protocol: everything this
user can do on the machine, here any TCP connection from it. The raw mode
listens wherever `--listen` does, 127.0.0.1 by default. Paths other than
`/socks` are the main protocol, as before. Sessions end when the server
reloads.

## SSH agent forwarding

What `ssh -A` does, over the tilr protocol: `ssh`, `git` or `ssh-add` in a
pane use the SSH agent of the user's computer, the client's side, and the
keys never leave it. On unless the server runs with `--no-agent` (or
`agent = false` in the configuration file).

- Every pane gets a Unix socket of its own, `agent/<pane id>.sock` in the
  runtime directory (`$XDG_RUNTIME_DIR/tilr`; on macOS `~/Library/Application
  Support/tilr`; `/tmp/tilr-<uid>` where that path is too long for a socket),
  and `SSH_AUTH_SOCK` pointing at it. The directory and the socket are the
  user's only. A [reload](#reloading-without-closing-shells) makes the socket
  again at the same path, so a shell's `SSH_AUTH_SOCK` stays good; the socket
  goes when the pane does. A program of another uid is refused, even where
  the permissions were opened by hand. Sockets a server killed with SIGKILL left behind
  are removed when the next one starts (those nobody accepts on: another
  running server's stay).
- A client that can serve the agent says `"agent"` in `Hello.features`; the
  server lists `"agent"` in `HelloAck.features` when forwarding is on.
- A program that connects to the socket gets a stream to one client: of the
  clients watching the pane that said `"agent"`, the one that subscribed
  last. The client gets `StreamOpen` (kind `"ssh-agent"`, the pane's id); the
  bytes go both ways as `StreamData` until either side sends `StreamClose`.
  The server only carries them.
- With no such client attached, the connection is closed at once: `ssh`
  finds an agent with no keys and goes on to its other ways in. A client
  that goes away takes its streams with it; so does a reload.
- A client has at most 64 streams open; a program sending faster than the
  client takes is held up (the server reads from it only as its client's
  [queue](#protocol) has room, 4 MiB for all its streams and file reads), a
  client sending faster than the program reads loses the stream.

As with `ssh -A`: while a client with the feature watches a pane, every
process of the user on the server's machine that finds a socket can use the
agent, for as many signatures as it asks. The agent on the client's side
asks for each signature if it is set up to (keys added with `ssh-add -c`,
say); otherwise turn forwarding off on servers you do not trust.

## Running programs (Exec)

A client can run a program on the server's machine without a pane or a
shell, as `ssh host cmd` would without the shell, and get its stdout and
stderr apart and how it ended. On unless the server runs with `--no-exec`
(or `exec = false` in the configuration file); the server lists `"exec"` in
`HelloAck.features`. It takes the same access token as everything else: a
client that may open a shell may run a program.

- `ExecStart` names the program and its arguments (`argv`), run as they
  are: no shell, nothing expanded; `argv[0]` is looked up in the server's
  `PATH`. Optionally a directory (`cwd`, else the home directory), `NAME=value`
  entries added to the server's environment, `stdin`, and a time limit
  (`timeout_ms`). The answer is `ExecStarted` with the exec's id and the
  program's pid, or with why it did not start (no such program or
  directory, more than 64 execs of the client running).
- The output comes as `ExecOutput` (stream 1 stdout, 2 stderr; at most
  64 KiB each, in order per stream), then `ExecExit`: the exit code, or the
  signal that killed the program, and whether it was for the time limit
  (`timed_out`) or `ExecCancel` (`cancelled`). Both go by the [second
  lane](#protocol): a client that takes them slowly holds the program up,
  blocked writing to its pipe, and the server's memory stays flat.
- `ExecInput` feeds stdin and closes it (`eof`). The server acknowledges
  what the program took with `ExecInputAck` (bytes in all); a client keeps
  at most 4 MiB ahead of that, and one that sends more has the exec killed
  (`reason` `"input overrun"`).
- `ExecSignal` sends a signal to the program's process group, as a terminal
  does for Ctrl+C; `ExecCancel` kills it at once.

The program leads a process group of its own, and nothing of it outlives
the exec: the whole group is killed (SIGKILL) on the time limit, on
`ExecCancel`, and when the program exits (a background child that keeps the
output open ends with it). Execs are not panes and do not survive a reload:
it kills them and their groups, and a client still connected gets
`ExecExit` with `reason` `"reload"` before its connection goes (`"shutdown"`
when the server stops). A client whose connection ends takes its execs with
it. On Linux a program is also killed should the server die without doing
it (`PR_SET_PDEATHSIG`); its children then go only if it had them in its
group. The log has each start and end with the client's id.

## Telling panes apart

A pane is named by a UUID the server makes up when the pane is created. It
names that pane and no other: not on another machine, and not after the server
is restarted, when a number would be handed out again.

The id is the same everywhere a pane shows up:

- in the protocol, as `pane_id`;
- in the process list: the pane process runs as `tilrd pane --id <id>`,
  followed by how the pane was started (`--cols` and `--rows` unless 80×24,
  `--cwd`, the command), and the shell is its child;
- inside the pane, as `$TILR_PANE`;
- in `tilr ps`, which asks the running server for its panes (`tilr ls` shows
  the first 8 characters, which `tilr attach` and `tilr kill` take):

```
$ tilr ps
PANE                                      PID     SIZE  VIEWERS  CWD  COMMAND
3b750286-ae31-4cc4-9edd-3f314fb00d57     8193    80x24        1  /home/dan  (shell)
d88b738a-fb48-41a5-bd4c-2050fdf33538     8219   120x40        0  /tmp  htop
```

`PID` is the pane process. `--connect ws://HOST:PORT` asks a server's TCP port
instead.

A server has an id of its own too, in `HelloAck.server_id`: a UUID made the
first time it runs on a machine and kept in `$XDG_STATE_HOME/tilr/server-id`
(`~/.local/state/tilr`), so it is the same after a restart.

## Protocol

Defined in [`crates/tilr-proto/proto/tilr.proto`](crates/tilr-proto/proto/tilr.proto).
A packet is one binary WebSocket message: a flag byte (0 = plain, 1 = zstd)
followed by a protobuf `ClientMessage` or `ServerMessage`.

The protocol grows by adding message types. Both sides ignore messages they do
not know, and say what they support in `Hello.features` / `HelloAck.features`.

A connection starts with `Hello`, which carries the protocol version of the
client and the [access token](#who-may-connect). A server that speaks another version, or is sent anything else first,
answers with an `Error` and closes the connection. `HelloAck` carries the
version of the server, and is the first message the client gets: nothing else
is sent to it before.

The server sends a WebSocket ping every third of `--client-timeout`. Browsers
and WebSocket libraries answer on their own. A client is dropped, and its
panes stop counting it as a viewer, when nothing came from it for the whole
timeout (it dropped off the network without closing the connection, or its
way to the server is cut), or when it still sends (something within the last
ping interval) but answered no ping for the whole timeout: it no longer hears
the server, its way from the server cut. What the client sends, `Ping`
included, does not keep a connection alive whose other direction is dead. The log says which (`client went silent`, `client stopped answering
pings`), and the server closes the connection with WebSocket close code 4002
(`timeout`), or resets it if not even the close gets out.

A client is one instance of an app (a window, a tab, a process), and says so
in `Hello.client_id`: a UUID v4 it makes up when it starts and sends on every
connection it makes. When a `Hello` names the `client_id` of a connection
still open, under the same access token, the server closes that older
connection at once with close code 4000 (`superseded`): it is the same client
come back (from another network, after a reload of its page), and the old
connection is what the network it left still holds, a ghost viewer otherwise
until the client timeout. The new connection starts afresh and subscribes
itself; nothing is carried over and no input is replayed (input sent into the
old connection may be lost, never doubled). A client does not reconnect a
connection closed as superseded. Another token with the same id supersedes
nobody; an empty `client_id` neither supersedes nor is superseded.
`PaneInfo.viewed_by` lists each viewer's `client_id`, name and address, and the
log has them on connect, hello, supersede and drop.

The server does not hoard frames for a client that cannot keep up. Unsent
data the kernel holds for a client is kept small (`TCP_NOTSENT_LOWAT`, 16 KiB),
so what it is behind on waits in its queue. A client whose queue has held a
packet for longer than 250 ms lags: its frames of a pane are skipped until
its queue is written out, then it gets a full frame of the screen as it is
then (the pane is asked for one; the other viewers get it too), whose
`last_input_seq` carries the client's own latest input if a skipped frame
echoed it. A client that skips gets fewer frames, each up to date, instead of
a growing backlog of stale ones; other clients are not slowed. A client whose
queue fills up all the same (1024 packets) is dropped (`client too slow`, close
code 4001), and its connection closed at once, whatever was queued.

Data in quantity does not wait in that queue. A client's queue has two lanes:
frames, answers and acknowledgements in one, and in the other file content
(`FsData`), the bytes of [streams](#ssh-agent-forwarding) (`StreamData`, and
the `StreamClose` after them) and the output of [execs](#running-programs-exec)
(`ExecOutput`, and the `ExecExit` after it). The connection writes from the second only when
the first is empty, so a keystroke's echo waits behind at most the one chunk
being written, not behind megabytes of files being read. Nor behind what the
network holds: of the second lane's data, at most 1 MiB is out on the way to
the client and not yet read by it (its receive buffer can hold several MB). The
server knows how far the client has read by WebSocket pings it sends between
the chunks, which the client's WebSocket answers as it reads; a client must
answer pings, as browsers and WebSocket libraries do on their own. The second
lane holds at most 4 MiB per client: a file read, a program sending to a
stream, or an exec's program writing output, waits for room, so a client that takes data slowly holds the read or
the program back, while the server's memory stays flat, and is not dropped for
being slow.

A page in a browser cannot see those pings, so for the other direction there
is `Ping` (feature `"ping"`): the server answers `Pong` with the same nonce at
once. A client that pings regularly and gets no answer in time knows the
server is out of reach, and reconnects.

### Keys

`KeyInput` names a key by what the layout makes of it (`codepoint`, the
unshifted character, and `text`, what it typed) and, in `code`, by the
physical key: the name W3C `KeyboardEvent.code` gives it (`"KeyC"`,
`"Digit1"`, `"BracketLeft"`, ...). A client sends both. In a browser they are
`event.key` and `event.code`; on macOS `NSEvent.charactersIgnoringModifiers`
and `NSEvent.keyCode` mapped to the code name (`kVK_ANSI_C` is `"KeyC"`); on
iPad `UIKey.charactersIgnoringModifiers` and `UIKey.keyCode`, a HID usage
(`UIKeyboardHIDUsageKeyboardC` is `"KeyC"`).

The physical key is what Ctrl and Alt go by on a layout that is not Latin,
as in other terminals: Ctrl+C on a Russian layout is the key that types "с",
and the program gets `^C` for it. When Ctrl or Alt is held and the key's
character is not ASCII, the server encodes the key as its character on a US
layout (Ctrl+С `^C`, Ctrl+Я `^Z`, Alt+Ф `ESC a`). A character that is ASCII
goes by itself wherever its key is: Ctrl+C on Dvorak (the I key) is `^C`,
Ctrl+A on AZERTY (the Q key) `^A`. Typing without Ctrl or Alt is the text.
With the kitty keyboard protocol on, the physical key is the base layout key
it reports. A client that sends no `code` gets what it did before: keys taken
as on a US layout, and nothing for Ctrl with a character that is not ASCII.

### File names that are not UTF-8

The files protocol (feature `"files"`) carries paths as strings, and a name
on Linux can be any bytes. A name that is not UTF-8 comes with its raw bytes
as well (`FsEntry.name_bytes`, `path_bytes` in `FsEntries`, `FsInfo`,
`FsChanged`), its string being for showing only (U+FFFD for each bad byte);
every request has the same `bytes` twin of its path (`path_bytes`, `from_bytes`,
`to_bytes`), which the server goes by when it is not empty. A client that
keeps the bytes it was given and sends them back can read, write, rename and
delete such a file; one that ignores them works as before for every other
name.

### Measuring input latency

The terminal pipeline can be measured end to end without synchronising
clocks. A client names an input message by `ClientMessage.seq` (outside the
`oneof`) on `KeyInput`, `MouseInput`, `Paste`, `RawInput`, `Scroll` or
`Resize`: `HelloAck.seq_base + n`, with `n` counting 1, 2, 3, ... on the
connection. `seq_base` is the connection's own (its number in the high 32
bits; numbers go on across a reload of the server), so a seq says which
connection's input it was, `seq_base <= seq < seq_base + 2^32`, even with
several clients on a pane or after a page reload.

Every `Frame` carries `last_input_seq`: the seq of the input the pane
credited last before it built the frame, from whichever client; 0 until there
is one. Key, mouse, paste and raw input is credited once the pty has taken its
last byte, in the order it came; `Scroll` and `Resize` when applied, which is
at once, so they may overtake key input still waiting for the pty. It is the
latest, not the highest: another client's input moves it, down as well. Every
credited input gets a frame. When it changed nothing on the screen (a click
the program ignores, a scroll that moves nothing, input not echoed), that
frame has no rows: `full` false, no `changed`, `styles`, `title` or
`images`, `placements_changed` false, and `cols`, `rows`, `cursor`,
`mouse_tracking`, `alt_screen`, the cell size and `viewport_top` as before, so
applying it changes nothing. Inputs credited between two frames show only the
last of them; a client takes a frame carrying a later seq of its own as the
echo of the earlier ones too.

The time from sending input `n` to the first frame carrying its seq (or a
later one of the same connection) is the latency the user sees, measured on
the client's clock alone. The frame that shows the echo usually comes a frame
or two later; `FrameTiming.output_bytes` tells which frames carry output.
Input without a seq makes no frame of its own. The fields are optional: a
server that does not know `seq` ignores it and sends `seq_base` 0, a client
that sends none gets 0. A client that ignores `seq_base` and numbers from 1
still gets its numbers echoed, but cannot tell them from another
connection's.

Every frame also carries `timing` (`FrameTiming`), the pane process's own
account, in microseconds:

| Field | What |
|---|---|
| `wait_us` | First pty output since the previous frame to the start of this frame's build: coalescing (frames are at least 8 ms apart) and scheduling. 0 for a frame no output caused. Output that changed nothing on the screen makes no frame and is counted in the next one, however much later |
| `parse_us` | Feeding the output into the screen model, summed |
| `build_us` | Building the frame from the screen model (diffing rows) |
| `encode_us` | Protobuf encoding and zstd compression of the pane's *previous* frame: a frame cannot hold its own |
| `output_bytes`, `reads` | Pty output since the previous frame, and the reads that brought it |
| `input_us` | For a new `last_input_seq`: the pane process receiving the input to the pty write that took its last byte returning (`Scroll`, `Resize`: to it being applied). 0 when no new input. Input still waiting for the pty when the server reloads keeps its time |

The supervisor's part (routing between client and pane) is in
[`/metrics`](#metrics).

## Metrics

`GET /metrics` on the server's port answers a plain HTTP request (not a
WebSocket) with the server's counters in the Prometheus text format
(`Content-Type: text/plain; version=0.0.4`). A request whose line starts with
`GET /metrics` followed by a space or `?` is taken for one, so `/metrics` and
`/metrics?...` are no longer paths a WebSocket can be opened on.

It asks for the access token as [`/socks`](#socks-proxy) does:
`Authorization: Bearer <token>`, or `?token=<token>`. Without it: 401; from a
browser page of a disallowed origin (`Origin`): 403. A server without a token
(`--no-auth`) asks for none, but answers only this machine: the `Host` header
must be loopback (`localhost`, `127.0.0.1` or `[::1]`, with or without a
port) or the address the server listens on (not `0.0.0.0`), and a request
with `Sec-Fetch-Site: cross-site` (a page of another site) is refused; 403
otherwise. A response not read within 5 s is dropped.

```sh
curl -H "Authorization: Bearer $(tilr token)" http://127.0.0.1:7890/metrics
```

| Metric | Type | What |
|---|---|---|
| `tilrd_info{version,server_id}` | gauge | Always 1 |
| `tilrd_clients` | gauge | Connected clients |
| `tilrd_panes` | gauge | Running panes |
| `tilr_pane_frames_total{pane}` | counter | Frames the pane produced |
| `tilr_pane_frame_bytes_total{pane}` | counter | Their bytes as routed, once per frame whatever the number of clients |
| `tilr_pane_frames_sent_total{pane}` | counter | Frames handed to clients, one per frame and client |
| `tilr_pane_frames_skipped_total{pane}` | counter | Frames not sent to a client that lagged (it got the whole screen later), one per frame and client |
| `tilr_client_drops_total{reason}` | counter | Clients the server let go: `superseded` by a newer connection of the same client, `too_slow`, `silent`, `no_pong` (sent, but did not answer pings) |
| `tilr_pane_input_total{pane}` | counter | Input packets (key, mouse, paste, raw, scroll) passed to the pane |
| `tilr_pane_input_bytes_total{pane}` | counter | Their bytes |
| `tilr_route_seconds` | histogram | A pane's packet, from the supervisor reading it off the pane process's pipe to the WebSocket write to the client returning (the client's queue included) |
| `tilr_route_input_seconds` | histogram | A client's input packet, from the supervisor reading it off the WebSocket to the write to the pane process's pipe returning |
| `tilr_supervisor_loop_lag_seconds` | histogram | How late the supervisor's event loop gets to a once-a-second tick. The tick comes from tokio's timer, which works in whole milliseconds: up to about 1 ms of every observation is the timer's, so the buckets under 1 ms say little |
| `tilr_process_cpu_seconds_total{role,pid,pane}` | counter | User and system CPU time of the supervisor (`role="supervisor"`, no `pane`) and of each pane process (`role="pane"`) |
| `tilr_process_memory_bytes{role,pid,pane}` | gauge | Their memory: resident set on Linux, physical footprint on macOS |

The histograms have buckets from 50 µs to 100 ms (0.00005, 0.0001, 0.00025,
0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, +Inf). The pane counters
start from zero when the supervisor does, a reload included. CPU and memory
are read when scraped. The pane process (the shell is its child) is what is
measured, not the programs in the pane.

## Pasting images: the shared clipboard

A program in a pane cannot reach the clipboard of the user's computer. When it
is asked to paste an image it reads the clipboard of the machine it runs on,
usually by running `xclip` or `wl-paste`, and over a plain SSH session finds
nothing there.

The server gives that machine a clipboard the client can fill:

1. The user pastes in the client. The client uploads the image with
   `ClipboardPut` (chunks of about 256 KB), then sends the paste key, Ctrl+V,
   to the pane.
2. The server stores the content, one clipboard for all panes.
3. The program in the pane runs `xclip` or `wl-paste`. The server has put its
   own versions of both first on the `PATH` the pane starts with; they answer
   from the stored content. The login profile keeps them when it adds to
   `PATH` (macOS `path_helper` puts the system directories in front and keeps
   the rest; `export PATH="...:$PATH"`); a profile that sets `PATH` from
   scratch (Debian's `/etc/profile`) drops them, and
   `PATH="$TILR_SHIMS:$PATH"` at the end of it brings them back.

So the program pastes as it would on a desktop. Verified against the commands
Claude Code 2.1.289 runs on Linux; they are spelled out in
`crates/tilrd/tests/common/mod.rs`.

Rules:

- The client need not wait for `ClipboardAck` before sending the key: the
  server handles messages of a connection in order.
- The first chunk of an upload empties the clipboard, so a failed upload never
  leaves an older image to be pasted by mistake.
- An upload over the limit is refused with a `ClipboardAck` carrying the
  reason. The limit is in `HelloAck.clipboard_max_bytes`; a client should
  check before sending and tell the user.
- Content expires after `--clipboard-ttl`.
- While the clipboard is empty, the server's `xclip` and `wl-paste` hand over
  to the real tools further down the `PATH`, if any. They also do for anything
  they do not serve: copying, the primary selection.

Limits of this approach:

- It serves programs that run `xclip` or `wl-paste`. A program that talks to
  X11 or Wayland directly does not see it.
- The clipboard is one per user and machine. Two clients pasting at the same
  moment overwrite each other.
- Linux only for now: on macOS, programs read the clipboard through other
  means.

Files live in `$XDG_RUNTIME_DIR/tilr` (or `tilr-<uid>` under the temp
directory), readable by the user only. To use the clipboard from a shell
outside tilr, say an SSH session on the same machine, put the `bin` directory
there first on its `PATH`.

## Reloading without closing shells

The server picks up a new executable without ending any shell:

- on its own, about a second after the file it was started from is replaced;
- on `SIGUSR1`.

The supervisor execs the new executable in place, keeping its pid, the
listening sockets (TCP, and the Unix socket with its lock) and the pane processes, then has every pane process do the
same. Shells, their screens, scrollback and scroll position carry over.

A process is exec'd with the arguments it was started with, so it looks the
same in `ps` before and after. What it hands to its successor goes into a file
in the runtime directory that the successor finds by name: `server-<pid>.state`
for the supervisor (an exec keeps the pid), `panes/<pane id>.state` for a pane.
The successor removes the file as it reads it. The files are readable by the
user only: a pane's holds its screen and the input not yet delivered. A state
file is written whole or not at all (a temporary file next to it, synced and
renamed): on a full disk the reload does not happen, that process carries on
as it was, and nothing half-written is left.

What to know:

- Client connections are dropped by a reload. Clients reconnect and find the
  panes as they were. A file write (`FsWrite`) cut by it fails and leaves the
  file as it was: each write notes its temporary file in `writes/` of the
  runtime directory, and a server that starts (or has just reloaded) removes
  the temporary files of writes whose process is gone or was itself.
- Install a new version by writing it next to the old one and renaming it into
  place (`install`, `mv`). The server reloads from the path it was started
  from; a version switch done by repointing a symlink is not noticed.
- Before reloading, the server asks the new file which versions of the reload
  state it reads (`tilrd state-formats`). If it does not run, or reads
  another version than the server writes, the server logs `not reloading` and
  carries on with the code it has. Such an upgrade needs a restart. An
  executable that does not say `unix-socket 1` there (one from before the
  socket) is not handed the socket: a server with a TCP port reloads into it
  without the socket, one on its socket alone does not reload.
- A pane process that cannot take its queued input within 5 seconds is closed
  rather than carried over.

## Tests

```sh
cargo test --workspace --features tilrd/failpoints
```

The tests in `crates/tilrd/tests/` start the real server and drive it as
a client would; `crates/tilr/tests/` run `tilr` against the `tilrd` built next
to it (so the tests of the workspace, not of `tilr` alone). [USE_CASES.md](USE_CASES.md) lists the use cases they cover —
connecting, reconnecting, scrolling, reloading, updating, the clipboard, the SSH agent, metrics — and
what is not covered yet.

The `failpoints` feature compiles in the places where a test can make an
update or a reload go wrong halfway: fail a step, crash or kill the process
there (`crates/tilrd/src/fail.rs`). Without it those tests are left out
and the rest runs. A release is built without it.
