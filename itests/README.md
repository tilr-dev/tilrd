# itests — black-box integration tests of tilrd, in Dagger

The real server binary in Linux containers on the pb Dagger engine, driven by a scripted
client, with faults injected between them. Every test runs in its own container,
concurrently. The catalog of use cases is [USE_CASES.md](USE_CASES.md) (decisions at the top).

| Kind | What | Id |
| --- | --- | --- |
| rust | each libtest test of the workspace (`crates/*/tests`, unit tests), built with `--features tilrd/failpoints` | `rust:<package>/<target>::<test>` |
| lint | `cargo clippy --workspace --all-targets -D warnings` (with failpoints), `cargo fmt --check` | `lint:clippy`, `lint:fmt` |
| fixture | building a prepared state (a cached snapshot) and checking it | `fixture:<name>` |
| scenario | each case of `scenarios/uc_*.py` | `scenario:uc_040:<case>` |

## Running

Needs the `dagger` CLI 0.21.10, `dagger-pb` (infra notes: «Dagger Engine на pb») and `uv`.

```sh
cd itests
./dt                                   # everything (~100 s warm)
./dt --only scenarios --filter uc_040  # one use case
./dt --only rust --filter reload       # some Rust tests
./dt --only scenarios --repeat 5       # every scenario five times: flakiness
./dt --only rust --filter edit --repeat 200   # Rust tests too
./dt --jobs 32                         # concurrency (default 16; the engine has 8 CPUs)
./dt --cached                          # rerun only tests whose inputs changed
./dt fixtures                          # fixture graph and how many cases use each
./dt fixture server_state              # export a snapshot to results/fixture-server_state
./dt call server export --path ./tilrd   # the Linux binary (also: driver, build)
```

Results go to `results/<timestamp>-<pid>/` (`results/latest` links the last run):
`summary.txt` (failures with the tail of their log, metrics, totals), `report.json`
(each test with status, duration and the metrics it recorded), `junit.xml`,
`logs/<test-id>.log`, `report.html` (failures first, filters, search) and `status`.
`results/index.html` lists every run and what broke or got fixed since the previous run
with the same arguments (`./dt history` opens it). `./dt clear [--keep N]` deletes local
reports. The exit code is non-zero if anything failed.

`DT_ENGINE=dagger` uses a local engine; `DT_DISTRO` picks the scenarios' runtime image
(only `ubuntu` = Ubuntu 24.04 so far). The commit of the checkout goes into the version
string of the build (`TILR_BUILD_COMMIT`).

The module lives in `itests/` of the tilrd repository; its context is the repository
root. Dagger 0.21 parses any `.env` above the module as dotenv: keep shell scripts out of
`.env` files there.

## What a run does

`src/tilr_itests/main.py`:

1. **build** — one Rust container (rust 1.94 bookworm, zig 0.15.2, protoc; `CARGO_HOME`
   and the target dirs are cache volumes on pb) builds the server (debug), `tilr-driver`
   (release: it plays a real client, and a debug one reads at ~30 MB/s), the test executables with failpoints (listing their tests), and the *next release*: the
   same source with `TILR_BUILD_VERSION=0.1.1`, in a target dir of its own so the two
   version strings never rebuild each other. Cached by Dagger until the source changes.
   Every build runs through `tt-fresh` (see [Fresh builds](#fresh-builds)). The last
   release named `tilr-server` is built once, from the tree of `PREVIOUS` in `dt`
   (`git archive` into `.previous/`, passed as `--previous`), in a target dir of its
   own, for the update across the rename (UC-06A); without that commit the case is
   BLOCKED.
2. **rust** — one runtime container per test, running the copied executable with
   `--exact NAME --test-threads=1`. Sources sit at `/src/tilrd`, the failpoints server at
   `/target/debug/tilrd`: the paths the tests baked in at compile time.
3. **fixtures** — each fixture is a Dagger layer: parents' `/fx` merged, then
   `python3 -m tt fixture MODULE:NAME` (build, then its mandatory check). *Source tier*
   fixtures have no binaries in the container and survive any code change; *tilr tier*
   (`tilr=True`) ones are rebuilt once per binary change.
4. **scenarios** — one container per case: the fixture snapshot copied into the writable
   layer at `/fx`, the binaries, the case's module, the services it names and its hosts
   (see Topology).
5. Every test gets a fresh `TT_RUN` id, so nothing is served from Dagger's cache unless
   `--cached`; the `test` function itself is `cache="never"`. A red test is data: the
   runner (`tt-run`) records exit code, duration, log and metrics in `/tt-result.json`,
   kills whatever the test left running, and never fails the exec.

Statuses: **PASS**; **FAIL** — the product disagrees with the use case, or the feature
does not exist yet; **BLOCKED** (exit 77) — the case needs something this backend does
not have, or its fixture failed. A test is never weakened to pass: a product defect stays
red and is described at the top of the case's module.

## Fresh builds

The target dirs are cache volumes on pb, shared by every run, worktree and commit, and
cargo decides what is up to date by mtimes: a source file newer than what was built
from it, or than the last run of a build script for its `rerun-if-changed` files. A tree
whose files are older than the last build of another tree (a worktree checked out
earlier, a commit built after a newer one) was taken as built: the run tested the other
tree's code, down to code generated from another `tilr.proto` (build.rs not rerun), and
a tree could fail to compile against it.

`tt-fresh DIR... -- CMD` (main.py `FRESH`) wraps every cargo call on a shared target
dir (`build`, the next release, the previous release, clippy and fmt): it takes the
dir's lock (concurrent runs of other trees wait), compares the sha256 of every source
file with `DIR/.tt-sources.json` (the tree the dir was last given), sets the mtime of
each file that differs to now, records the new tree and runs CMD. A changed file is
then always newer than anything built before it; an unchanged one is rebuilt only if
cargo thinks so. The record is written before CMD, so a build that fails halfway still
counts as given that tree.

Checked by building three trees in a row with `./dt call --commit X --source DIR server`:
A, then B (A with one more value in `enum FsChange` of tilr.proto and nothing else),
then A again; the debug binary of each has `TtCacheProbe` in its debug info only from B
(A: 0, B: 1, A: 0). Without `tt-fresh` the shared cache already held code generated
from an older proto and A did not compile (`no variant ExecCancel`).

## Layout

```
itests/
  dt                     wrapper around dagger-pb
  src/tilr_itests/       the Dagger module
  tt/                    the helper (stdlib only, Python 3.12 in the containers)
    __init__.py          @fixture, @case, T, Server, Driver, Session, Proc
    __main__.py          python3 -m tt list | fixture M:NAME | case uc_NNN:NAME
    proxy.py             the layer-1 fault proxy (also a standalone process / service)
    host.py              the agent of a host container (commands, a driver over TCP)
    live.py              live software in panes for the update and reload cases (group F)
    cloud.py             tilr Cloud played by a case, the stand-in cloudflared
  fixtures/*.py          prepared states shared by cases
  scenarios/uc_NNN.py    the cases of one use case
  tools/                 report.html and the history page
```

### Container layout (backend `dagger`)

```
/fx                       fixture snapshot, the case's working directory
/tmp/tt                   scratch: each server's runtime/state/config dirs and log, driver stderr
tilrd, tilr-driver        on PATH; root's login shell is bash with PS1='$ '
/opt/tt/tilrd-next        the next release; /opt/tt/manifest.json its version and platform
/opt/tt/tilr-server-previous  the last release named tilr-server, as 0.0.9 (UC-06A)
$TT_SERVICE_CDN           http://cdn:8080, the fake CDN, for cases with services=["cdn"]
```

## tilr-driver

`crates/tilr-driver`: a client with one long-lived WebSocket connection, JSON lines in
(commands) and out (events). Every event has `ev` and `t` (ms since the driver started,
monotonic); a command's answer carries its `id` and `cmd`; a failed command answers
`{"ev": "error", "message"}` and the driver carries on. It keeps a plain-text model of
every subscribed pane's screen from the frames.

| Command | Arguments | Answer |
| --- | --- | --- |
| `connect` | `url`, `token`, `origin`, `features` (default `["ping"]`), `timeout_ms`, `client_id` (`Hello.client_id`; default: the driver's own, made up when it starts and kept across its reconnects) | `connected` with `server_version`, `features`, `seq_base`, `fs_max_bytes`, `clipboard_max_bytes`, `client_id` |
| `create_pane` | `cols`, `rows`, `cwd`, `command`, `subscribe` (default true) | `pane_created` with `pane`, `info` |
| `subscribe` / `unsubscribe` / `close_pane` | `pane` | `ok` |
| `input` / `paste` | `pane`, `text` (raw bytes, escapes are JSON's) | `ok` with the input `seq` |
| `key` | `pane`, `key`: `enter`, `esc`, `up`, `f5`, `ctrl-c`, `alt-x`, `shift-tab`, `q`, any character (`ctrl-с`: Cyrillic, as a Russian layout sends it); several space-separated | `ok` |
| `mouse` | `pane`, `action` (`press`, `release`, `motion`), `button` (`left`, `right`, `middle`, `wheel_up`, `wheel_down`, `none`), `x`, `y` (cells from 0), `mods` (bits: 1 shift, 2 alt, 4 ctrl) | `ok` with the input `seq` |
| `resize` | `pane`, `cols`, `rows` | `ok` |
| `screen` | `pane` | `screen`: `rows`, `text`, `cursor`, `cursor_visible`, `alt_screen`, `mouse_tracking`, `title`, `frames` |
| `wait_for` | `pane`, `regex` (on the screen text, rows joined by `\n`), `timeout_ms` | `matched` with `groups`, or an error with the screen |
| `list` | | `panes` (each with `viewers` and `viewed_by`: `client_id`, `name`, `addr` per viewer) |
| `ping` | `timeout_ms` | `pong` with `rtt_ms` |
| `keepalive` | `interval_ms` (0 = off), `timeout_ms`, `auto_reconnect`, `reconnect_timeout_ms`, `attempt_timeout_ms` (one reconnect attempt, 3000), `mode` (`any`: any data is a sign of life; `pong`: only answers to its pings), `abandon` (`close`; `leak`: a dead connection is left open and unread, like a suspended app) | `ok` |
| `reconnect` | `timeout_ms` | `reconnected` with `ms`, `attempts`, `server_version` |
| `drop` / `disconnect` | | `ok` (drop: socket gone without a close, like the network) |
| `sleep` | `ms` (keeps reading meanwhile) | `ok` |
| `status` | | `connected`, `subscribed` |
| `fs_write` | `path`, content (`text`, `from`: a local file read as the chunks go, or `size` bytes of `fill` repeated), `chunk` (65536), `pause_ms` between chunks, `create`, `truncate` (both true), `wait` (true), `background`, `timeout_ms` | `fs_ack` with `error` ("" = done, or why the server refused; the rest of a refused write is not sent), `sent`, `size`, `chunks`, `sha256` of what was sent, `ms`; an error if the connection is lost on the way (needs `features` with `files`). `background`: `fs_started` with `transfer` at once, the chunks go between everything else the driver does, `fs_done` says how it ended |
| `fs_read` | `path`, `to` (a local file; without, the answer's `text` has the first 64 KiB), `offset`, `length` (0 = to the end), `part` (bytes one FsRead asks for; 0 = all in one, refused over `fs_max_bytes`), `background`, `timeout_ms` | `fs_data` with `error`, `size`, `file_size`, `parts`, `sha256`, `ms` (`background`: as fs_write) |
| `fs_wait` | `transfer`, `timeout_ms` | `fs_done`: the outcome of a background transfer (as fs_write's / fs_read's answer, `lost` if its connection went) |
| `fs_list` / `fs_stat` | `path` | `fs_entries` with `path`, `entries`, `error` / `fs_info` with `path`, `entry`, `error`; an entry is `name`, `kind` and `target` (`file`, `dir`, `link`, `other`), `size`, `mtime_ms`, `mode` |
| `clipboard_put` | `mime` (text/plain), content as fs_write, `chunk`, `total_size` (what the first chunk announces; default the real size), `timeout_ms` | `clipboard_ack` with `ok`, `error`, `size`, `sha256` |
| `agent` | `mode`: `echo` (a toy SSH agent that echoes every byte back; the default), `sink` (take and hash, answer nothing), `socket` with `socket`: a local agent's path, each stream relayed to it as `ssh -A` does; `echo` true/false = echo/sink | `ok` |
| `streams` / `stream_close` | `stream` | the open streams (`stream`, `kind`, `pane`, `bytes_in`, `bytes_out`, `sha256_in`) / `ok` |
| `stream_send` | `stream`, content as fs_write (64 KiB per StreamData) | `ok` with `sent`, `sha256` |
| `socks_open` | `host`, `port` on the server's machine, `url` (default: the connection's `/socks?token=`), `timeout_ms` | `socks_open` with `socks` (an id) |
| `socks_send` / `socks_recv` / `socks_close` | `socks`; send: content as `fs_write`; recv: `bytes` wanted, `timeout_ms` | recv: `socks_data` with `received`, `text`, `ended` |
| `socks_listen` | `addr` (127.0.0.1:0), `url` (default: the connection's `/socks?token=`) | `socks_listening` with `port`: a local SOCKS5 port for programs on the client's machine (curl), each connection carried to the server's /socks as it is |
| `socks_stats` | | `socks_stats`: `sessions`, `open`, `failed`, `bytes_up`, `bytes_down` of the local SOCKS ports |
| `exec` | `argv`, `cwd`, `env` (`["NAME=value"]`), `stdin` (text, fed and closed; `null`: a pipe fed by exec_input), `limit_ms` (the program's time limit), `background`, `timeout_ms` | `exec_exit` with `code`, `signal`, `timed_out`, `cancelled`, `reason`, `stdout`, `stderr` (256 KiB of each), `*_bytes`, `*_sha256`, `ms`; `background`: `exec_started` with `exec`. Only to a server with the feature "exec"; refused (`ExecStarted.error`): `error` |
| `exec_input` / `exec_signal` / `exec_cancel` / `exec_wait` | `exec`; input: content as fs_write, sent in 64 KiB pieces at most 4 MiB ahead of `ExecInputAck`, `eof`; signal: `signal` (15) | `ok` / `ok` / `ok` / `exec_exit` |
| `latency` | `reset` | `latency`: `count`, `pending`, `unanswered`, `p50_ms` `p90_ms` `p99_ms` `max_ms` of input → first frame carrying its seq (`Frame.last_input_seq`) |
| `frames` | `since` (index), `pane`, `reset` | `frames`: the last 20000 frames (`t`, `pane`, `seq`, `full`, `rows`, `bytes`, `input_seq`, the pane's `FrameTiming`), `seen`, `bytes`, `max_gap_ms` |
| `quit` | | `bye` |

Unsolicited events: `disconnected` (`reason`; `superseded`: the server closed it with 4000
because a newer connection of the same client id took over, and it is not reconnected), `dead` (keepalive saw `timeout_ms` of
silence), `reconnected` / `reconnect_failed` (auto reconnect; resubscribes), `pane_exited`,
`server_error`, `protocol_error`, `stream_open` / `stream_closed` (with `sha256_in`) /
`stream_lost` (agent streams; a stream is lost with its connection), `host_event` (`pane`,
`name`, `kind`, `path`, `args`: `tilr edit` / `tilr open` in a pane, to a client with
"host-events"), `fs_done` (a background transfer ended), `exec_exit` (a background exec). Screens go stale when the connection is
lost: `screen` and `wait_for` wait for the full frame of the next connection.

## Writing a case

```python
from tt import T, case

@case(fixture="server_state")
def a_reconnecting_client_finds_its_shell(t: T):
    """UC-0NN: one line about the behavior."""
    server = t.server(state=t.root / "state")
    d = t.driver()
    d.connect(server)
    pane = d.shell_pane()
    d.line(pane, "export KEPT=yes")
    d.cmd("reconnect")
    t.eq(d.value(pane, "$KEPT"), "yes", "the variable")
```

* Name the case after the behavior, as a sentence; the docstring starts with the UC id.
  One case = one independent subcase, starting from a fresh copy of its fixture.
* A scenario module imports only `tt` (only that file is mounted).
* If the product disagrees with the use case, the case fails — note at the top of the
  module what is observed and where. An unimplemented feature is written against the
  intended interface and fails until it exists.
* Many independent outcomes of one setup: `t.check("what", fn)` records a failure and goes
  on; the case fails at the end with all of them.
* Measurements: `t.metric("name_ms", value)` — they land in report.json and the summary.
* Things only a container gives (procfs, a second host, root): `t.need("procfs")` so the
  case is BLOCKED, not failed, on the coming `local` backend.

### Helper (`tt.T`)

| | |
| --- | --- |
| `server(*args, exe=, state=, env=, listen=)` | start tilrd with its own dirs; waits for `listening on`; `.url .port .pid .token() .cmd(...) .version() .log_text() .logged(s) .alive() .panes() .stop()` |
| `found_server(log, dir, env)` | a tilrd started some other way (from a session), found by its log |
| `install(exe=)` | a copy of the server in a writable dir (for updates) |
| `driver(name)` | a tilr-driver: `.cmd(name, **args)`, `.connect(server)`, `.shell_pane()`, `.line()`, `.key()`, `.wait_for()`, `.screen()`, `.text()`, `.value(pane, "$X")`, `.run(pane, cmd)` → exit status, `.list()`, `.wait_event(ev, since=d.mark())`; events carry `_at` (the case's clock) |
| `session(*argv, env=)` | a terminal login session (pty, setsid, controlling tty): `.write() .expect(re) .hangup()` |
| `proxy(port)` | the layer-1 fault proxy in front of a port (below): `.url .freeze() .resume() .cut() .set() .stats()` |
| `proxy_service()` | the same proxy in a container of its own (`services=["proxy"]`); `.target(host, port)` first |
| `host(name)` | a host container (`hosts=[...]`): `.ip .peer_ip` (the case container as the host reaches it) `.run() .sh() .driver(name) .net` |
| `net()` | `Net` of the case's own container (`net_admin=True`) |
| `Counter(t, driver, pane, period)` | a counter in a pane: `.last(d) .check_screen(d)` (moving, lines consecutive) `.check_log()` (the pane's own log: nothing skipped, longest gap as `counter_max_gap_ms`) |
| driver extras | `.interrupt(pane)` (Ctrl-C until the prompt is back), `.viewers(pane)` (as the server counts them), `.latency(reset)`, `.connected()`, `.wait_event(("dead", "disconnected"), since=)` |
| `service(name)` | URL of a service container named in `@case(services=[...])` |
| `cdn()` | `LocalCdn`: releases served from the case's own container, `.release(version, exe)`, `.latest(version)`, `.url` for `update --cdn` (an older release, a broken one) |
| `cloud()` | tilr Cloud played by the case (tt/cloud.py): `.link(state_dir)`, `.accept()` → a control connection: `.greet()`, `.send(msg)`, `.next()`, `.tunnel_is(state)`; `fake_cloudflared(dir)` notes `args`, `config`, `pid`, `ended` there |
| `tmpfs(path, size)` | a small disk to fill (needs `net_admin=True`: mount wants the root capabilities) |
| `tt.live` | `LivePanes(t, d, server, echo=)`: a shell with a variable and a job, a counter, vim (unsaved, mouse on), htop, less, a python REPL, `sleep 1000`, a TCP connection to the echo service; `.check(what)` after an update or reload (pids, screens and modes, no orphans), `.finish()` (`:wq`, `q`, Ctrl+C); `wait_reloads`, `reload_times`, `state_files` |
| `run(*argv)`, `sh(script)`, `spawn(*argv)` | processes (missing executable = exit 127) |
| `procs(regex)`, `proc(pid)`, `listener_pid(port)` | /proc: pid, ppid, pgid, sid, tty, state, cmdline, rss_kb, fds |
| `eq`, `ok`, `has`, `lacks`, `eventually(fn, what, timeout)`, `check(what, fn)` | assertions |
| `metric(name, value)`, `need(*caps)`, `scratch(name)`, `manifest()` | |

### A fixture

```python
from tt import T, fixture

@fixture(requires=["files"], tilr=True)
def server_state(t: T):
    """/fx/state: TILR_STATE_DIR with the access token."""
    t.run("tilrd", "token", env={"TILR_STATE_DIR": str(t.root / "state")}, check=True)

@server_state.check
def _(t: T):
    ...
```

Every fixture must have a check (the registry refuses one without). A failed fixture
marks its cases BLOCKED. Add a fixture only when two or more cases need the state.

### The fault proxy (layer 1)

`tt/proxy.py`, a TCP relay that understands WebSocket framing, in the case's process
(`t.proxy(port)`) or as a service (`services=["proxy"]`, `t.proxy_service()`, JSON lines on
its control port). Connections are numbered from 1 as accepted.

| | |
| --- | --- |
| `freeze(direction="both", conn=None)` | stop reading and passing on (`up` client → server, `down`, `both`): the silent DROP of a network; held bytes go on at `resume`, as TCP retransmits. `conn`: one connection (`1`), or `"open"` (those open now, not later ones); default: all, new ones included |
| `resume(direction, conn)` | the same choice |
| `cut(conn=None)` | RST both ways |
| `set(dir="both", **knobs)` | `delay_ms`, `jitter_ms` (per chunk, order kept); `loss_pct`, `loss_stall_ms` (packet loss as a TCP stream shows it: the chunk waits a retransmission timeout, 200 ms doubling per loss in a row; nothing dropped, doubled or reordered — real loss, duplicates and reordering are netem's); `rate_bps`, `buffer_bytes` (bandwidth; 64 KiB held at most); `cut_after_ws_messages`, `cut_after_bytes`, `cut_mid_ws_message` (RST, once). The protocol compresses: cut by messages, not bytes |
| `stats()` | per connection: `peer`, `closed`, `read_*` and `bytes_*` (written) per direction, `ws_up` / `ws_down` messages |

### Topology

By default a case is one container: tilrd, the drivers and the proxy talk over loopback.
A case that needs other IPs or real network faults declares them:

* `hosts=["wifi", "lte"]` — a container per name (Dagger service, the binaries, root with
  NET_ADMIN), running `tt/host.py`; `t.host(name)` drives it: commands, and drivers whose
  stdin/stdout cross a TCP connection. The server listens on `0.0.0.0` and a host reaches
  the case's container by IP (`host.peer_ip`): Dagger containers share a bridge. The
  case's services are bound into its hosts too (one instance: equal services run once).
* `services=["proxy"]` — the proxy as a container of its own: client, proxy and tilrd in
  three containers (UC-042).
* `net_admin=True` — the case's own exec gets NET_ADMIN (insecure root capabilities), for
  `t.net()`.

Layer 2, `Net` (`host.net`, `t.net()`), on the container's own interface (`eth1` in
Dagger, found from the default route):

| | |
| --- | --- |
| `netem(spec, dst=None, port=None)` | `tc qdisc ... netem <spec>` on egress, e.g. `"delay 150ms 20ms loss 1% reorder 10% 50%"`; with `dst` and `port`, only the TCP packets to `dst` of connections with that port at either end (a prio qdisc and u32 filters), so control traffic is not shaped. Both directions: one on the host, one on the case (`net_admin`). `ifb` for ingress also works on pb |
| `drop(peer, port, direction="both")` | `iptables -j DROP` in a chain `TT`, `in` (from the peer's port) / `out` / both: half-open |
| `reject(peer, port)` | `REJECT --reject-with tcp-reset` |
| `restore()` | flush the chain: the network is back |
| `kill_sockets(peer, port)` | `ss -K`: the kernel destroys the sockets and resets the peer |
| `clear_netem()` | |

All of it is undone when the case ends.

### Services

A case asks for a service container by name: `@case(services=["cdn"])`. The module
builds it (`_service` in main.py), binds it under that hostname and passes its URL as
`TT_SERVICE_<NAME>`. `cdn` serves `releases/<platform>/latest` and the next release with
its checksum, as `tilrd update` reads them. `echo` is a TCP echo service on port
7007 (`tcp://echo:7007`), the far end of a pane's connection that must outlive a reload.

## Gotchas

* Hundreds of `with_new_file` in one Directory break the overlay mount on export: the
  report is written by a single exec from a JSON bundle.
* Files of a lower overlay layer cannot be renamed or hard-linked: the fixture snapshot is
  copied into the container's writable layer before a case.
* Parallel cargo in different containers: the whole `CARGO_HOME` is one cache volume.
* Test executables bake paths in: `/src/tilrd`, `/target` are the same everywhere.
* `build.rs` reruns when `.git/logs/HEAD` is missing, which rebuilds tilrd on every
  cargo call; the build makes an empty one with an old mtime.
* The runner writes the test's output to a file, not a pipe: a daemon the case leaves
  running would hold a pipe open until the timeout.
* Matching the screen: the command line you typed is on the screen too. Wait for what
  only the program prints (`echo "<v""1>..."` is typed split for that reason; vim's
  `"file" [New]`, not the file name).
* UC-040's server-side drop time varies between `client-timeout − ping interval` and
  `client-timeout`: the server counts from the client's last pong, not from the moment
  the network froze.
* The interface of a Dagger container is `eth1`, not `eth0`.
* `ss -K dst <case>` and `iptables` rules on the case's whole address also kill the
  host agent's own control connection: match the server's port.
* Dagger runs equal service definitions once per session: a stateful service (the proxy,
  a host) carries the case run's id in its environment (`TT_OWNER`).
* `FsWrite` chunks and frames are compressed: 4 MB of a repeated pattern is 3.5 KB on the
  wire, so cut a transfer by messages, not bytes.
* Text typed while a pane's foreground program runs waits in the tty, and Ctrl-C flushes
  it: input whose effect a case counts goes to an idle shell pane.
* TCP at 30% loss each way: retransmission backoff stalls for tens of seconds and a
  reconnect can take a minute; UC-043 asserts what the server owes and records the rest.
