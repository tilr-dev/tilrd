# tilr

A terminal server. It runs shells in panes on a machine and serves them to
clients over a WebSocket: a client gets the screen as a grid of cells and sends
keys back. Panes keep running when clients disconnect.

Status: early (see [TODO.md](TODO.md)).

## Layout

| Crate | What it is |
|---|---|
| `tilr-proto` | The wire protocol: `proto/tilr.proto` and packet framing |
| `tilr-screen` | Terminal screen model (libghostty-vt) that produces frames |
| `tilr-server` | The server binary |

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
`tilr-server`, without the JavaScript `tilr` CLI. It installs a user LaunchAgent,
then updates the managed server through its own signed server feed, separately
from the application's Sparkle updates. A newer compatible server survives app
upgrades with older bundled copies. Use **Tilr > Check for Server Updates** for
this managed copy; separate installations retain their own updater.
See [Mac release and lifecycle details](../apps/mac/PRODUCTION.md).

Releases are built for Linux on aarch64 (glibc 2.36 or newer). There is no
installer yet: download the binary and start it.

```sh
version=$(curl -fsSL https://cdn.tilr.dev/releases/linux-aarch64/latest)
curl -fsSLo tilr-server "https://cdn.tilr.dev/releases/$version/tilr-server-linux-aarch64"
chmod +x tilr-server
./tilr-server
```

To update, in any shell on that machine:

```sh
tilr-server update
```

It asks the CDN which version is ready for its platform, downloads it, checks
it against its checksum and that it runs, and puts it in place of the
executable. A running server then [reloads itself](#reloading-without-closing-shells)
and keeps every shell. `--cdn URL` (or `TILR_CDN`) names another place to
fetch releases from.

`tilr-server --version` prints what a build is: the package version, the
commit and the platform, e.g. `0.1.0-26449dd (linux-aarch64)`.

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

## Running

```sh
tilr-server                       # listens on ws://127.0.0.1:7890
```

| Flag | Default | Meaning |
|---|---|---|
| `--listen ADDR` | `127.0.0.1:7890` | WebSocket address |
| `--allow-origin ORIGIN` | the configuration file's list, else `tilr://app` (the Tilr app's page), `tilr://tilr` (its terminal app) and the web client's | Browser origins allowed to connect; repeatable |
| `--config PATH` | `~/.config/tilr/server.toml` (`$XDG_CONFIG_HOME/tilr/server.toml`), if there | The [configuration file](#the-configuration-file) |
| `--no-auth` | | Do not ask clients for the [access token](#who-may-connect) |
| `--cloudflared PATH` | `cloudflared` | The `cloudflared` to run for a [tunnel](#tilr-cloud) |
| `--no-clipboard` | | Turn the shared clipboard off |
| `--clipboard-max SIZE` | `20MB` | Largest clipboard upload (`512K`, `20MB`, `1G`) |
| `--clipboard-ttl SECS` | `300` | How long uploaded content stays; `0` = until replaced |
| `--client-timeout SECS` | `30` | Drop a client that stays silent this long; `0` = never |
| `--no-auto-reload` | | Do not reload when the executable changes |
| `--socks` | the configuration file's `socks`, else off | Serve the [SOCKS proxy](#socks-proxy) on the same port |
| `--no-agent` | | Turn [SSH agent forwarding](#ssh-agent-forwarding) off (also `agent = false`) |

Signals: `SIGTERM` and `SIGINT` stop the server and end its shells; `SIGUSR1`
[reloads](#reloading-without-closing-shells) it. `SIGHUP` is ignored: a server
started from a terminal (`tilr-server &` in an SSH session) outlives it, and so
do its panes, each in a session of its own. Its log then goes nowhere; redirect
stderr to keep it.

## Who may connect

A client must send the server's access token in `Hello.token`. The server
makes the token the first time it runs and keeps it in
`$XDG_STATE_HOME/tilr/token` (`~/.local/state/tilr`), readable by the user
only. To see it, on that machine:

```sh
tilr-server token
```

A client without the token, or with another, gets an `Error` and the
connection is closed. `tilr-server ps` reads the token itself.

`--no-auth` turns the check off: then anyone who can reach the port gets a
shell. A page in a browser must, with or without the token, come from an
allowed origin: `--allow-origin`, else the configuration file's list, else
the built-in one.

## The configuration file

What a machine's operator sets once lives in `~/.config/tilr/server.toml`
(`$XDG_CONFIG_HOME/tilr/server.toml`, or `--config PATH` / `TILR_CONFIG`),
read when the server starts (a reload starts it again, so a change takes
effect on the next `tilr-server update` or SIGUSR1). A missing file is an
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
```

## tilr Cloud

Optional. Cloud lists the machines of a user, says which are online, and gives
each a tunnel: a name on the internet that leads to the server's port, so a
phone can reach a machine that accepts no incoming connections. What goes
through the tunnel does not pass through Cloud.

```sh
tilr-server cloud login      # prints a link and a code to confirm in a browser
tilr-server cloud status
tilr-server cloud logout     # Cloud forgets the machine and removes its tunnel
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
curl --socks5-hostname "x:$(ssh my-server tilr-server token)@127.0.0.1:1080" http://pb.localhost:3000
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
  goes when the pane does.
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
  client takes is held up, a client sending faster than the program reads
  loses the stream.

As with `ssh -A`: while a client with the feature watches a pane, every
process of the user on the server's machine that finds a socket can use the
agent, for as many signatures as it asks. The agent on the client's side
asks for each signature if it is set up to (keys added with `ssh-add -c`,
say); otherwise turn forwarding off on servers you do not trust.

## Telling panes apart

A pane is named by a UUID the server makes up when the pane is created. It
names that pane and no other: not on another machine, and not after the server
is restarted, when a number would be handed out again.

The id is the same everywhere a pane shows up:

- in the protocol, as `pane_id`;
- in the process list: the pane process runs as `tilr-server pane --id <id>`,
  followed by how the pane was started (`--cols` and `--rows` unless 80×24,
  `--cwd`, the command), and the shell is its child;
- inside the pane, as `$TILR_PANE`;
- in `tilr-server ps`, which asks a running server for its panes:

```
$ tilr-server ps
PANE                                      PID     SIZE  VIEWERS  CWD  COMMAND
3b750286-ae31-4cc4-9edd-3f314fb00d57     8193    80x24        1  /home/dan  (shell)
d88b738a-fb48-41a5-bd4c-2050fdf33538     8219   120x40        0  /tmp  htop
```

`PID` is the pane process. `--url` names a server listening elsewhere than
`ws://127.0.0.1:7890`.

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
and WebSocket libraries answer on their own. A client that stays silent for
the whole timeout has dropped off the network without closing the connection:
the server drops it, and its panes stop counting it as a viewer.

A page in a browser cannot see those pings, so for the other direction there
is `Ping` (feature `"ping"`): the server answers `Pong` with the same nonce at
once. A client that pings regularly and gets no answer in time knows the
server is out of reach, and reconnects.

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
curl -H "Authorization: Bearer $(tilr-server token)" http://127.0.0.1:7890/metrics
```

| Metric | Type | What |
|---|---|---|
| `tilr_server_info{version,server_id}` | gauge | Always 1 |
| `tilr_server_clients` | gauge | Connected clients |
| `tilr_server_panes` | gauge | Running panes |
| `tilr_pane_frames_total{pane}` | counter | Frames the pane produced |
| `tilr_pane_frame_bytes_total{pane}` | counter | Their bytes as routed, once per frame whatever the number of clients |
| `tilr_pane_frames_sent_total{pane}` | counter | Frames handed to clients, one per frame and client |
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
`crates/tilr-server/tests/common/mod.rs`.

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
listening socket and the pane processes, then has every pane process do the
same. Shells, their screens, scrollback and scroll position carry over.

A process is exec'd with the arguments it was started with, so it looks the
same in `ps` before and after. What it hands to its successor goes into a file
in the runtime directory that the successor finds by name: `server-<pid>.state`
for the supervisor (an exec keeps the pid), `panes/<pane id>.state` for a pane.
The successor removes the file as it reads it. The files are readable by the
user only: a pane's holds its screen and the input not yet delivered.

What to know:

- Client connections are dropped by a reload. Clients reconnect and find the
  panes as they were.
- Install a new version by writing it next to the old one and renaming it into
  place (`install`, `mv`). The server reloads from the path it was started
  from; a version switch done by repointing a symlink is not noticed.
- Before reloading, the server asks the new file which versions of the reload
  state it reads (`tilr-server state-formats`). If it does not run, or reads
  another version than the server writes, the server logs `not reloading` and
  carries on with the code it has. Such an upgrade needs a restart.
- A pane process that cannot take its queued input within 5 seconds is closed
  rather than carried over.

## Tests

```sh
cargo test --workspace --features tilr-server/failpoints
```

The tests in `crates/tilr-server/tests/` start the real server and drive it as
a client would. [USE_CASES.md](USE_CASES.md) lists the use cases they cover —
connecting, reconnecting, scrolling, reloading, updating, the clipboard, the SSH agent, metrics — and
what is not covered yet.

The `failpoints` feature compiles in the places where a test can make an
update or a reload go wrong halfway: fail a step, crash or kill the process
there (`crates/tilr-server/src/fail.rs`). Without it those tests are left out
and the rest runs. A release is built without it.
