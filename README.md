# tilr

A terminal server. It runs shells in panes on a machine and serves them to
clients over a WebSocket: a client gets the screen as a grid of cells and sends
keys back. Panes keep running when clients disconnect.

Status: early. There is no authentication yet, so the server must stay on
loopback (see [TODO.md](TODO.md)).

## Layout

| Crate | What it is |
|---|---|
| `tilr-proto` | The wire protocol: `proto/tilr.proto` and packet framing |
| `tilr-screen` | Terminal screen model (libghostty-vt) that produces frames |
| `tilr-server` | The server binary |

One supervisor process accepts clients and routes packets. Each pane is its
own process, a child of the supervisor, which owns the pty, the shell and the
screen model.

## Installing and updating

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
releases on the CDN is described there.

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
scripts/linux.sh                  # cargo test --workspace
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
| `--allow-origin ORIGIN` | the web client's | Browser origins allowed to connect; repeatable |
| `--no-clipboard` | | Turn the shared clipboard off |
| `--clipboard-max SIZE` | `20MB` | Largest clipboard upload (`512K`, `20MB`, `1G`) |
| `--clipboard-ttl SECS` | `300` | How long uploaded content stays; `0` = until replaced |
| `--client-timeout SECS` | `30` | Drop a client that stays silent this long; `0` = never |
| `--no-auto-reload` | | Do not reload when the executable changes |

## Protocol

Defined in [`crates/tilr-proto/proto/tilr.proto`](crates/tilr-proto/proto/tilr.proto).
A packet is one binary WebSocket message: a flag byte (0 = plain, 1 = zstd)
followed by a protobuf `ClientMessage` or `ServerMessage`.

The protocol grows by adding message types. Both sides ignore messages they do
not know, and say what they support in `Hello.features` / `HelloAck.features`.

A connection starts with `Hello`, which carries the protocol version of the
client. A server that speaks another version, or is sent anything else first,
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
   own versions of both first on the pane's `PATH`; they answer from the
   stored content.

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
cargo test --workspace
```

The tests in `crates/tilr-server/tests/` start the real server and drive it as
a client would. [USE_CASES.md](USE_CASES.md) lists the use cases they cover —
connecting, reconnecting, scrolling, reloading, the clipboard — and what is
not covered yet.
