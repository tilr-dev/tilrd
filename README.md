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
| `--no-auto-reload` | | Do not reload when the executable changes |

## Protocol

Defined in [`crates/tilr-proto/proto/tilr.proto`](crates/tilr-proto/proto/tilr.proto).
A packet is one binary WebSocket message: a flag byte (0 = plain, 1 = zstd)
followed by a protobuf `ClientMessage` or `ServerMessage`.

The protocol grows by adding message types. Both sides ignore messages they do
not know, and say what they support in `Hello.features` / `HelloAck.features`.

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
`crates/tilr-server/tests/use_cases.rs`.

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
same. Shells, their screens and scrollback carry over.

What to know:

- Client connections are dropped by a reload. Clients reconnect and find the
  panes as they were.
- Install a new version by writing it next to the old one and renaming it into
  place (`install`, `mv`). The server reloads from the path it was started
  from; a version switch done by repointing a symlink is not noticed.
- Before reloading, the server runs the new file with `--version`. If that
  fails, it logs `not reloading` and carries on with the code it has.
- A pane process that cannot take its queued input within 5 seconds is closed
  rather than carried over.

## Tests

```sh
cargo test --workspace
```

`crates/tilr-server/tests/use_cases.rs` starts the real server and drives it
as a client would: running commands in a pane, pasting an image and reading it
back with the commands Claude Code uses, the size limit, expiry, and reloads
by signal and by replacing the executable.
