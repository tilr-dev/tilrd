# itests — black-box integration tests of tilrd, in Dagger

The real server binary in Linux containers on the pb Dagger engine, driven by a scripted
client, with faults injected between them. Every test runs in its own container,
concurrently. The catalog of use cases is [USE_CASES.md](USE_CASES.md) (decisions at the top).

| Kind | What | Id |
| --- | --- | --- |
| rust | each libtest test of the workspace (`crates/*/tests`, unit tests), built with `--features tilr-server/failpoints` | `rust:<package>/<target>::<test>` |
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
./dt --jobs 32                         # concurrency (default 16; the engine has 8 CPUs)
./dt --cached                          # rerun only tests whose inputs changed
./dt fixtures                          # fixture graph and how many cases use each
./dt fixture server_state              # export a snapshot to results/fixture-server_state
./dt call server export --path ./tilr-server   # the Linux binary (also: driver, build)
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
   and the target dirs are cache volumes on pb) builds the server and `tilr-driver`, the
   test executables with failpoints (listing their tests), and the *next release*: the
   same source with `TILR_BUILD_VERSION=0.1.1`, in a target dir of its own so the two
   version strings never rebuild each other. Cached by Dagger until the source changes.
2. **rust** — one runtime container per test, running the copied executable with
   `--exact NAME --test-threads=1`. Sources sit at `/src/tilrd`, the failpoints server at
   `/target/debug/tilr-server`: the paths the tests baked in at compile time.
3. **fixtures** — each fixture is a Dagger layer: parents' `/fx` merged, then
   `python3 -m tt fixture MODULE:NAME` (build, then its mandatory check). *Source tier*
   fixtures have no binaries in the container and survive any code change; *tilr tier*
   (`tilr=True`) ones are rebuilt once per binary change.
4. **scenarios** — one container per case: the fixture snapshot copied into the writable
   layer at `/fx`, the binaries, the case's module and the services it names.
5. Every test gets a fresh `TT_RUN` id, so nothing is served from Dagger's cache unless
   `--cached`; the `test` function itself is `cache="never"`. A red test is data: the
   runner (`tt-run`) records exit code, duration, log and metrics in `/tt-result.json`,
   kills whatever the test left running, and never fails the exec.

Statuses: **PASS**; **FAIL** — the product disagrees with the use case, or the feature
does not exist yet; **BLOCKED** (exit 77) — the case needs something this backend does
not have, or its fixture failed. A test is never weakened to pass: a product defect stays
red and is described at the top of the case's module.

## Layout

```
itests/
  dt                     wrapper around dagger-pb
  src/tilr_itests/       the Dagger module
  tt/                    the helper (stdlib only, Python 3.12 in the containers)
    __init__.py          @fixture, @case, T, Server, Driver, Session, Proc
    __main__.py          python3 -m tt list | fixture M:NAME | case uc_NNN:NAME
    proxy.py             the layer-1 fault proxy (also a standalone process)
  fixtures/*.py          prepared states shared by cases
  scenarios/uc_NNN.py    the cases of one use case
  tools/                 report.html and the history page
```

### Container layout (backend `dagger`)

```
/fx                       fixture snapshot, the case's working directory
/tmp/tt                   scratch: each server's runtime/state/config dirs and log, driver stderr
tilr-server, tilr-driver  on PATH; root's login shell is bash with PS1='$ '
/opt/tt/tilr-server-next  the next release; /opt/tt/manifest.json its version and platform
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
| `connect` | `url`, `token`, `origin`, `features` (default `["ping"]`), `timeout_ms` | `connected` with `server_version`, `features`, `seq_base` |
| `create_pane` | `cols`, `rows`, `cwd`, `command`, `subscribe` (default true) | `pane_created` with `pane`, `info` |
| `subscribe` / `unsubscribe` / `close_pane` | `pane` | `ok` |
| `input` / `paste` | `pane`, `text` (raw bytes, escapes are JSON's) | `ok` with the input `seq` |
| `key` | `pane`, `key`: `enter`, `esc`, `up`, `f5`, `ctrl-c`, `alt-x`, `q`; several space-separated | `ok` |
| `resize` | `pane`, `cols`, `rows` | `ok` |
| `screen` | `pane` | `screen`: `rows`, `text`, `cursor`, `alt_screen`, `title`, `frames` |
| `wait_for` | `pane`, `regex` (on the screen text, rows joined by `\n`), `timeout_ms` | `matched` with `groups`, or an error with the screen |
| `list` | | `panes` |
| `ping` | `timeout_ms` | `pong` with `rtt_ms` |
| `keepalive` | `interval_ms` (0 = off), `timeout_ms`, `auto_reconnect`, `reconnect_timeout_ms` | `ok` |
| `reconnect` | `timeout_ms` | `reconnected` with `ms`, `attempts`, `server_version` |
| `drop` / `disconnect` | | `ok` (drop: socket gone without a close, like the network) |
| `sleep` | `ms` (keeps reading meanwhile) | `ok` |
| `quit` | | `bye` |

Unsolicited events: `disconnected` (reason), `dead` (keepalive saw `timeout_ms` of
silence), `reconnected` / `reconnect_failed` (auto reconnect; resubscribes), `pane_exited`,
`server_error`, `protocol_error`.

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
| `proxy(port)` | the fault proxy in front of a port: `.freeze(dir) .resume() .cut() .set(delay_ms=, jitter_ms=, cut_after_ws_messages=) .stats()` |
| `service(name)` | URL of a service container named in `@case(services=[...])` |
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
    t.run("tilr-server", "token", env={"TILR_STATE_DIR": str(t.root / "state")}, check=True)

@server_state.check
def _(t: T):
    ...
```

Every fixture must have a check (the registry refuses one without). A failed fixture
marks its cases BLOCKED. Add a fixture only when two or more cases need the state.

### Services

A case asks for a service container by name: `@case(services=["cdn"])`. The module
builds it (`_service` in main.py), binds it under that hostname and passes its URL as
`TT_SERVICE_<NAME>`. `cdn` serves `releases/<platform>/latest` and the next release with
its checksum, as `tilr-server update` reads them.

## Gotchas

* Hundreds of `with_new_file` in one Directory break the overlay mount on export: the
  report is written by a single exec from a JSON bundle.
* Files of a lower overlay layer cannot be renamed or hard-linked: the fixture snapshot is
  copied into the container's writable layer before a case.
* Parallel cargo in different containers: the whole `CARGO_HOME` is one cache volume.
* Test executables bake paths in: `/src/tilrd`, `/target` are the same everywhere.
* `build.rs` reruns when `.git/logs/HEAD` is missing, which rebuilds tilr-server on every
  cargo call; the build makes an empty one with an old mtime.
* The runner writes the test's output to a file, not a pipe: a daemon the case leaves
  running would hold a pipe open until the timeout.
* Matching the screen: the command line you typed is on the screen too. Wait for what
  only the program prints (`echo "<v""1>..."` is typed split for that reason; vim's
  `"file" [New]`, not the file name).
* UC-040's server-side drop time varies between `client-timeout − ping interval` and
  `client-timeout`: the server counts silence from the client's last message (its last
  pong), not from the moment the network froze.
