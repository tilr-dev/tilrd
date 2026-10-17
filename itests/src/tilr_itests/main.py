"""Black-box integration tests of tilrd, the server binary, run natively in Dagger.

Every test runs in its own container, concurrently:

* rust     — each libtest test of the workspace, built with the failpoints feature;
* lint     — clippy -D warnings and rustfmt --check;
* fixture  — building a prepared state (a cached snapshot) and checking its invariants;
* scenario — each case of itests/scenarios/uc_*.py: the real server, the tilr-driver
             client, a fault-injecting proxy, service containers (a fake CDN), in a
             runtime image of a distribution (Ubuntu LTS for now).

The build runs once per source change: the server (plain and with failpoints), the
driver, the test executables and a "next release" — the same source with another
version string — for the update scenarios; and, once, the last release named
tilr-server (the source ./dt passes as --previous) for the update across the rename.
"""

import asyncio
import dataclasses
import datetime
import json
import uuid
from typing import Annotated

import dagger
from dagger import DefaultPath, Doc, Ignore, dag, function, object_type

RUST_IMAGE = "rust:1.94.0-bookworm"
ZIG_VERSION = "0.15.2"
# Runtime images of the distributions the scenarios run on; one for now.
DISTROS = {"ubuntu": "ubuntu:24.04"}
RUNTIME_APT = ["python3", "procps", "psmisc", "util-linux", "bash", "ca-certificates", "curl",
               "vim", "htop", "less", "iproute2", "lsof", "iptables", "openssh-client", "openssh-server",
               "git"]

# Same paths in every container: test executables bake CARGO_MANIFEST_DIR and
# CARGO_BIN_EXE_* at compile time.
SRC = "/src/tilrd"
TARGET = "/target"
# Version string of the "next release" the update scenarios install.
NEXT_PACKAGE_VERSION = "0.1.1"
# Version string of the last release named tilr-server, built from --previous.
PREVIOUS_PACKAGE_VERSION = "0.0.9"

SOURCE_IGNORE = ["*", "!Cargo.toml", "!Cargo.lock", "!crates/", "crates/*/target/", "!rustfmt.toml",
                 "!.rustfmt.toml", "!clippy.toml"]
ITESTS_IGNORE = ["sdk/", ".venv/", "results/", ".previous/", "**/__pycache__/", "src/"]

# Runs one test command and writes a single JSON result, so the module reads one file
# per test. The test's exit code never fails the exec: a red test is data, not an error.
# Output goes to a file, not a pipe: daemons a scenario leaves behind would hold a pipe
# open. Everything the test started is killed afterwards.
RUNNER = r'''#!/usr/bin/env python3
import json, os, signal, subprocess, sys, time

def pids():
    return {int(p) for p in os.listdir("/proc") if p.isdigit()}

timeout = int(os.environ.get("TT_TIMEOUT", "600"))
before = pids()
start = time.monotonic()
with open("/tt-log.txt", "wb") as out:
    p = subprocess.Popen(sys.argv[1:], stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                         start_new_session=True)
    try:
        code = p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        code = 124
        out.write(f"\n[tt] timeout after {timeout}s\n".encode())
ms = int((time.monotonic() - start) * 1000)
for pid in pids() - before - {os.getpid()}:
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
log = open("/tt-log.txt", "rb").read().decode("utf-8", "replace")
if len(log) > 400_000:
    log = log[:200_000] + "\n[tt] ... log truncated ...\n" + log[-200_000:]
metrics = {}
if os.path.exists("/tt-metrics.json"):
    try:
        metrics = json.load(open("/tt-metrics.json"))
    except ValueError:
        pass
with open("/tt-result.json", "w") as f:
    json.dump({"code": code, "ms": ms, "log": log, "metrics": metrics}, f)
'''

# tt-fresh DIR... -- CMD: run CMD holding the lock of each target dir, after giving every
# source file whose content differs from what that target dir was last built from the
# mtime of now. Cargo decides what to rebuild by mtimes (a source newer than its output,
# a build script's rerun-if-changed file newer than its last run), and the target dirs
# are cache volumes shared by every checkout and run: a source tree whose files are
# older than the last build of another tree (another commit, worktree or run) would be
# taken as built and tested as the other tree, down to code generated from another
# tilr.proto. The digests of the tree a target dir was last given are in
# DIR/.tt-sources.json, written before CMD runs: a build that fails halfway has still
# been given that tree.
FRESH = r'''#!/usr/bin/env python3
import fcntl, hashlib, json, os, subprocess, sys
split = sys.argv.index("--")
dirs, argv = sys.argv[1:split], sys.argv[split + 1:]

def digests():
    out = {}
    for root, subdirs, files in os.walk("."):
        subdirs[:] = [d for d in subdirs if d not in ("target", ".git")]
        for name in files:
            path = os.path.join(root, name)[2:]
            with open(path, "rb") as f:
                out[path] = hashlib.sha256(f.read()).hexdigest()
    return out

now = digests()
locks = []
for d in sorted(dirs):
    os.makedirs(d, exist_ok=True)
    lock = open(f"{d}/.tt-fresh.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    locks.append(lock)
    try:
        with open(f"{d}/.tt-sources.json") as f:
            before = json.load(f)
    except (OSError, ValueError):
        before = {}
    changed = [p for p, digest in now.items() if before.get(p) != digest]
    for path in changed:
        os.utime(path)
    print(f"[tt-fresh] {d}: {len(changed)} of {len(now)} source files changed since its last build",
          file=sys.stderr, flush=True)
    with open(f"{d}/.tt-sources.json.tmp", "w") as f:
        json.dump(now, f)
    os.replace(f"{d}/.tt-sources.json.tmp", f"{d}/.tt-sources.json")
sys.exit(subprocess.run(argv).returncode)
'''

# Builds everything once: the server and the driver, the test executables (with
# failpoints) and their test names, and the next release from another target dir so
# that switching version strings never rebuilds the main one.
BUILD = r'''#!/usr/bin/env python3
import json, os, shutil, subprocess
out = "/out"
for d in ("tests", "bin"):
    os.makedirs(f"{out}/{d}")
# build.rs watches .git/logs/HEAD; without it the build script reruns on every cargo call.
os.makedirs(".git/logs", exist_ok=True)
if not os.path.exists(".git/logs/HEAD"):
    open(".git/logs/HEAD", "w").close()
    os.utime(".git/logs/HEAD", (0, 0))

def cargo(*args, env=None, **kw):
    return subprocess.run(["cargo", *args], check=True, env=dict(os.environ, **(env or {})), **kw)

cargo("build", "--locked", "-p", "tilrd")
shutil.copy2("/target/debug/tilrd", f"{out}/bin/tilrd")
# The driver plays a real client, which is an optimized build: a debug one hashes, decodes
# and writes files at ~30 MB/s, and what it is slow to read queues in its receive buffer.
cargo("build", "--locked", "--release", "-p", "tilr-driver")
shutil.copy2("/target/release/tilr-driver", f"{out}/bin/tilr-driver")

p = cargo("test", "--no-run", "--locked", "--workspace", "--features", "tilrd/failpoints",
          "--message-format=json-render-diagnostics", stdout=subprocess.PIPE, text=True)
suites = []
for line in p.stdout.splitlines():
    if not line.startswith("{"):
        continue
    m = json.loads(line)
    if m.get("reason") != "compiler-artifact" or not m.get("executable") or not m["profile"]["test"]:
        continue
    package = os.path.basename(os.path.dirname(m["manifest_path"]))
    kind = m["target"]["kind"][0]
    suite = f"{package}/{kind}" if kind in ("lib", "bin") else f"{package}/{m['target']['name']}"
    exe = f"{out}/tests/{suite.replace('/', '__')}"
    shutil.copy2(m["executable"], exe)
    listed = subprocess.run([exe, "--list", "--format", "terse"], stdout=subprocess.PIPE, check=True, text=True)
    names = [l[: -len(": test")] for l in listed.stdout.splitlines() if l.endswith(": test")]
    suites.append({"suite": suite, "exe": exe, "manifest_dir": os.path.dirname(m["manifest_path"]), "tests": names})
# What CARGO_BIN_EXE_tilrd of the tests points at: the failpoints build.
shutil.copy2("/target/debug/tilrd", f"{out}/bin/tilrd-failpoints")

cargo("build", "--locked", "-p", "tilrd",
      env={"CARGO_TARGET_DIR": "/target-next", "TILR_BUILD_VERSION": os.environ["TT_NEXT_PACKAGE_VERSION"]})
shutil.copy2("/target-next/debug/tilrd", f"{out}/bin/tilrd-next")

def version(exe):
    words = subprocess.run([exe, "--version"], stdout=subprocess.PIPE, check=True, text=True).stdout.split()
    return words[1], words[2].strip("()")

current, platform = version(f"{out}/bin/tilrd")
nxt, _ = version(f"{out}/bin/tilrd-next")
with open(f"{out}/manifest.json", "w") as f:
    json.dump({"suites": suites, "version": current, "next_version": nxt, "platform": platform}, f, indent=1)
'''

# Builds the last release named tilr-server from its own source (--previous).
BUILD_PREVIOUS = r'''#!/usr/bin/env python3
import os, shutil, subprocess
os.makedirs(".git/logs", exist_ok=True)
if not os.path.exists(".git/logs/HEAD"):
    open(".git/logs/HEAD", "w").close()
    os.utime(".git/logs/HEAD", (0, 0))
subprocess.run(["cargo", "build", "--locked", "-p", "tilr-server"], check=True)
os.makedirs("/out")
shutil.copy2("/target/debug/tilr-server", "/out/tilr-server-previous")
'''

# Lays out a CDN as crates/tilrd/src/update.rs reads it, offering the next release under
# both names, as scripts/release.sh publishes it.
CDN = r'''#!/usr/bin/env python3
import hashlib, json, os, shutil
m = json.load(open("/build/manifest.json"))
version, platform = m["next_version"], m["platform"]
os.makedirs(f"/cdn/releases/{platform}", exist_ok=True)
os.makedirs(f"/cdn/releases/{version}", exist_ok=True)
for name in (f"tilrd-{platform}", f"tilr-server-{platform}"):
    shutil.copy2("/build/bin/tilrd-next", f"/cdn/releases/{version}/{name}")
    digest = hashlib.sha256(open(f"/cdn/releases/{version}/{name}", "rb").read()).hexdigest()
    open(f"/cdn/releases/{version}/{name}.sha256", "w").write(f"{digest}  {name}\n")
open(f"/cdn/releases/{platform}/latest", "w").write(version + "\n")
'''

# A TCP echo service on port 7007, a thread per connection: the far end of a pane's
# outbound connection that must outlive a reload of the server.
ECHO = r'''#!/usr/bin/env python3
import socket, threading
def serve(conn):
    with conn:
        while data := conn.recv(65536):
            conn.sendall(data)
srv = socket.create_server(("0.0.0.0", 7007), reuse_port=True)
while True:
    conn, _ = srv.accept()
    threading.Thread(target=serve, args=(conn,), daemon=True).start()
'''

# A deterministic interactive shell in the panes: plain prompt, no history file.
BASHRC = """PS1='$ '
PS2='> '
HISTFILE=/dev/null
unset PROMPT_COMMAND
"""


@dataclasses.dataclass
class Result:
    id: str
    kind: str
    status: str  # PASS | FAIL | BLOCKED
    ms: int = 0
    log: str = ""
    metrics: dict = dataclasses.field(default_factory=dict)


@object_type
class TilrItests:
    source: Annotated[dagger.Directory, Doc("tilrd checkout (the workspace root)"), DefaultPath("/"),
                      Ignore(SOURCE_IGNORE)]
    itests: Annotated[dagger.Directory, Doc("tt helper, fixtures and scenarios"), DefaultPath("/itests"),
                      Ignore(ITESTS_IGNORE)]
    commit: Annotated[str, Doc("commit baked into the version string (TILR_BUILD_COMMIT)")] = "itest"
    distro: Annotated[str, Doc(f"runtime image of the scenarios: {', '.join(DISTROS)}")] = "ubuntu"
    previous: Annotated[dagger.Directory | None,
                        Doc("source of the last release named tilr-server (./dt passes it); without it "
                            "the update across the rename is BLOCKED"), Ignore(SOURCE_IGNORE)] = None
    previous_commit: Annotated[str, Doc("the commit of --previous, baked into its version string")] = "previous"

    # ---------------------------------------------------------------- containers

    def _rust(self, target_cache: str, source: dagger.Directory | None = None) -> dagger.Container:
        return (
            dag.container()
            .from_(RUST_IMAGE)
            .with_exec(["sh", "-c", "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "
                        "--no-install-recommends protobuf-compiler git xz-utils ca-certificates curl procps python3 "
                        ">/dev/null"])
            .with_exec(["sh", "-c", f"curl -fsSL https://ziglang.org/download/{ZIG_VERSION}/zig-$(uname -m)-linux-"
                        f"{ZIG_VERSION}.tar.xz | tar -xJ -C /opt && ln -s /opt/zig-$(uname -m)-linux-{ZIG_VERSION}/zig "
                        "/usr/local/bin/zig"])
            .with_exec(["rustup", "component", "add", "clippy", "rustfmt"])
            # The whole CARGO_HOME, so cargo's package-cache lock is shared by concurrent containers.
            .with_mounted_cache("/cargo-home", dag.cache_volume("tt-cargo-home"))
            .with_env_variable("CARGO_HOME", "/cargo-home")
            .with_mounted_cache(TARGET, dag.cache_volume(target_cache))
            .with_env_variable("CARGO_TARGET_DIR", TARGET)
            .with_env_variable("CARGO_TERM_COLOR", "never")
            .with_env_variable("TILR_BUILD_COMMIT", self.commit)
            .with_new_file("/usr/local/bin/tt-fresh", FRESH, permissions=0o755)
            .with_directory(SRC, source or self.source)
            .with_workdir(SRC)
        )

    @function
    def runtime(self) -> dagger.Container:
        """Runtime image of the scenarios and the Rust tests, with the result runner."""
        if self.distro not in DISTROS:
            raise ValueError(f"unknown distro {self.distro}; known: {', '.join(DISTROS)}")
        return (
            dag.container()
            .from_(DISTROS[self.distro])
            .with_exec(["sh", "-c", "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "
                        f"--no-install-recommends {' '.join(RUNTIME_APT)} >/dev/null"])
            .with_new_file("/root/.bashrc", BASHRC)
            .with_new_file("/root/.bash_profile", ". ~/.bashrc\n")
            .with_env_variable("LANG", "C.UTF-8")
            .with_new_file("/usr/local/bin/tt-run", RUNNER, permissions=0o755)
        )

    @function
    def build(self) -> dagger.Directory:
        """bin/ (tilrd, tilrd-failpoints, tilrd-next, tilr-driver), tests/ and
        manifest.json (test suites, versions, platform)."""
        return (
            self._rust("tt-target")
            .with_mounted_cache("/target-next", dag.cache_volume("tt-target-next"))
            .with_env_variable("TT_NEXT_PACKAGE_VERSION", NEXT_PACKAGE_VERSION)
            .with_new_file("/usr/local/bin/tt-build", BUILD, permissions=0o755)
            .with_exec(["tt-fresh", TARGET, "/target-next", "--", "tt-build"])
            .directory("/out")
        )

    @function
    def previous_server(self) -> dagger.File:
        """The last release named tilr-server, as version 0.0.9 (needs --previous)."""
        if self.previous is None:
            raise ValueError("no --previous source")
        return (
            self._rust("tt-target-previous", self.previous)
            .with_env_variable("TILR_BUILD_VERSION", PREVIOUS_PACKAGE_VERSION)
            .with_env_variable("TILR_BUILD_COMMIT", self.previous_commit)
            .with_new_file("/usr/local/bin/tt-build-previous", BUILD_PREVIOUS, permissions=0o755)
            .with_exec(["tt-fresh", TARGET, "--", "tt-build-previous"])
            .file("/out/tilr-server-previous")
        )

    @function
    def server(self) -> dagger.File:
        """The tilrd binary (Linux, debug, no failpoints)."""
        return self.build().file("bin/tilrd")

    @function
    def driver(self) -> dagger.File:
        """The tilr-driver binary."""
        return self.build().file("bin/tilr-driver")

    def _with_binaries(self, ctr: dagger.Container) -> dagger.Container:
        out = self.build()
        ctr = (ctr.with_file("/usr/local/bin/tilrd", out.file("bin/tilrd"))
                .with_file("/usr/local/bin/tilr-driver", out.file("bin/tilr-driver"))
                .with_file("/opt/tt/tilrd-next", out.file("bin/tilrd-next"))
                .with_file("/opt/tt/manifest.json", out.file("manifest.json")))
        if self.previous is not None:
            ctr = ctr.with_file("/opt/tt/tilr-server-previous", self.previous_server())
        return ctr

    async def _run(self, sem: asyncio.Semaphore, ctr: dagger.Container, argv: list[str],
                   rid: str, kind: str, net_admin: bool = False) -> Result:
        async with sem:
            try:
                raw = await (ctr.with_exec(["tt-run", *argv], insecure_root_capabilities=net_admin)
                             .file("/tt-result.json").contents())
            except dagger.ExecError as e:
                return Result(rid, kind, "FAIL", 0, f"[tt] runner failed:\n{e.stdout}\n{e.stderr}")
            except dagger.QueryError as e:
                return Result(rid, kind, "FAIL", 0, f"[tt] {e}")
        r = json.loads(raw)
        status = "PASS" if r["code"] == 0 else "BLOCKED" if kind == "scenario" and r["code"] == 77 else "FAIL"
        return Result(rid, kind, status, r["ms"], r["log"], r.get("metrics") or {})

    # ---------------------------------------------------------------- rust, lint

    async def _rust_tests(self, sem, run_id: str, filter: str, repeat: int = 1) -> list[Result]:
        out = self.build()
        manifest = json.loads(await out.file("manifest.json").contents())
        base = (
            self.runtime()
            .with_directory(SRC, self.source)
            .with_directory("/out/tests", out.directory("tests"))
            .with_file(f"{TARGET}/debug/tilrd", out.file("bin/tilrd-failpoints"))
        )
        jobs = []
        for s in manifest["suites"]:
            for name in s["tests"]:
                rid = f"rust:{s['suite']}::{name}"
                if filter and filter not in rid:
                    continue
                for k in range(1, repeat + 1):
                    ctr = base.with_workdir(s["manifest_dir"]).with_env_variable("TT_RUN", f"{run_id}-{k}")
                    jobs.append(self._run(sem, ctr, [s["exe"], "--exact", name, "--test-threads=1"],
                                          rid + (f"#{k}" if repeat > 1 else ""), "rust"))
        return list(await asyncio.gather(*jobs))

    async def _lint(self, sem, run_id: str, filter: str) -> list[Result]:
        checks = [
            ("lint:clippy", "tt-target-clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets",
                                                 "--features", "tilrd/failpoints", "--", "-D", "warnings"]),
            ("lint:fmt", "tt-target-fmt", ["cargo", "fmt", "--all", "--check"]),
        ]
        jobs = []
        for rid, cache, argv in checks:
            if filter and filter not in rid:
                continue
            ctr = (self._rust(cache)
                   .with_new_file("/usr/local/bin/tt-run", RUNNER, permissions=0o755)
                   .with_env_variable("TT_RUN", run_id))
            jobs.append(self._run(sem, ctr, ["tt-fresh", TARGET, "--", *argv], rid, "lint"))
        return list(await asyncio.gather(*jobs))

    # ---------------------------------------------------------------- fixtures

    def _tt(self, ctr: dagger.Container, *modules: str) -> dagger.Container:
        """Mount the helper and only the given modules: unrelated edits keep the caches."""
        ctr = ctr.with_directory("/itests/tt", self.itests.directory("tt"))
        for m in modules:
            path = m.replace(".", "/") + ".py"
            ctr = ctr.with_file(f"/itests/{path}", self.itests.file(path))
        return (ctr.with_env_variable("PYTHONPATH", "/itests")
                .with_env_variable("PYTHONDONTWRITEBYTECODE", "1")
                .with_env_variable("TT_BACKEND", "dagger")
                .with_env_variable("TT_DISTRO", self.distro)
                .with_env_variable("HOME", "/root")
                .with_workdir("/fx"))

    async def _registry(self) -> dict:
        ctr = (self.runtime().with_directory("/itests", self.itests)
               .with_env_variable("PYTHONPATH", "/itests").with_env_variable("PYTHONDONTWRITEBYTECODE", "1"))
        return json.loads(await ctr.with_exec(["python3", "-m", "tt", "list"]).stdout())

    def _fixture_builder(self, registry: dict):
        """name -> Task[Directory]; each fixture is built once per test run."""
        tasks: dict[str, asyncio.Task] = {}
        fixtures = registry["fixtures"]

        async def build(name: str) -> dagger.Directory:
            spec = fixtures[name]
            fx = dag.directory()
            for parent in spec["requires"]:
                fx = fx.with_directory(".", await get(parent))
            ctr = self._tt(self.runtime(), spec["module"])
            if spec["tilr"]:
                ctr = self._with_binaries(ctr)
            ctr = ctr.with_directory("/fx", fx).with_exec(
                ["python3", "-m", "tt", "fixture", f"{spec['module']}:{name}"])
            await ctr.sync()
            return ctr.directory("/fx")

        def get(name: str) -> asyncio.Task:
            if name not in tasks:
                tasks[name] = asyncio.ensure_future(build(name))
            return tasks[name]

        return get

    async def _fixture_results(self, get, registry: dict, names) -> list[Result]:
        out = []
        for name in names:
            try:
                await get(name)
                out.append(Result(f"fixture:{name}", "fixture", "PASS"))
            except dagger.ExecError as e:
                failed_parent = any(_failed(get, p) for p in registry["fixtures"][name]["requires"])
                if failed_parent:
                    out.append(Result(f"fixture:{name}", "fixture", "BLOCKED", 0, "a required fixture failed"))
                else:
                    out.append(Result(f"fixture:{name}", "fixture", "FAIL", 0, f"{e.stdout}\n{e.stderr}"))
            except Exception as e:
                out.append(Result(f"fixture:{name}", "fixture", "BLOCKED", 0, _error_text(e)))
        return out

    # ---------------------------------------------------------------- services

    def _service(self, name: str, owner: str) -> tuple[dagger.Service, str]:
        """A service container a case asks for by name, and the URL it is reached at.
        `owner` makes a stateful service (the proxy) the case's own: Dagger shares
        services whose definitions are equal."""
        if name == "proxy":
            svc = (self._tt(self.runtime())
                   .with_env_variable("TT_OWNER", owner)
                   .with_exposed_port(9000)
                   .with_exposed_port(9001)
                   .as_service(args=["python3", "-m", "tt.proxy", "--listen", "0.0.0.0:9000",
                                     "--control", "0.0.0.0:9001"]))
            return svc, "tcp://proxy:9000"
        if name == "cdn":
            out = self.build()
            cdn = (self.runtime()
                   .with_directory("/build", out)
                   .with_new_file("/usr/local/bin/tt-cdn", CDN, permissions=0o755)
                   .with_exec(["tt-cdn"])
                   .directory("/cdn"))
            svc = (self.runtime()
                   .with_directory("/cdn", cdn)
                   .with_exposed_port(8080)
                   .as_service(args=["python3", "-m", "http.server", "8080", "--directory", "/cdn"]))
            return svc, "http://cdn:8080"
        if name == "echo":
            svc = (self.runtime()
                   .with_new_file("/usr/local/bin/tt-echo", ECHO, permissions=0o755)
                   .with_exposed_port(7007)
                   .as_service(args=["tt-echo"]))
            return svc, "tcp://echo:7007"
        raise ValueError(f"unknown service {name}")

    def _host(self, owner: str, services: dict[str, dagger.Service]) -> dagger.Service:
        """A container of its own a case drives (tt/host.py): the binaries, root with
        NET_ADMIN for netem and iptables on its own interface, the case's services bound
        (the same instances: Dagger runs equal services once)."""
        ctr = self._with_binaries(self._tt(self.runtime()))
        for name, svc in services.items():
            ctr = ctr.with_service_binding(name, svc)
        return (ctr.with_env_variable("TT_OWNER", owner)
                .with_exposed_port(7000)
                .as_service(args=["python3", "-m", "tt.host", "--port", "7000"], insecure_root_capabilities=True))

    # --------------------------------------------------------------- scenarios

    async def _scenarios(self, sem, run_id: str, filter: str, registry: dict, repeat: int = 1) -> list[Result]:
        get = self._fixture_builder(registry)
        cases = [c for c in registry["cases"] if not filter or filter in f"scenario:{c['id']}"]
        needed: set[str] = set()

        def closure(n):
            if n and n not in needed:
                needed.add(n)
                for p in registry["fixtures"][n]["requires"]:
                    closure(p)

        for c in cases:
            closure(c["fixture"])

        async def one(c, k: int) -> Result:
            rid = f"scenario:{c['id']}" + (f"#{k}" if repeat > 1 else "")
            fx = dag.directory()
            if c["fixture"]:
                try:
                    fx = await get(c["fixture"])
                except Exception:
                    return Result(rid, "scenario", "BLOCKED", 0, f"fixture {c['fixture']} failed to build")
            ctr = (self._with_binaries(self._tt(self.runtime(), c["module"]))
                   .with_directory("/fx", fx)
                   .with_env_variable("TT_RUN", f"{run_id}-{k}")
                   .with_env_variable("TT_TIMEOUT", str(c["timeout"])))
            owner = f"{run_id}-{k}-{c['id']}"
            services = {}
            for name in c["services"]:
                svc, url = self._service(name, owner)
                services[name] = svc
                ctr = (ctr.with_service_binding(name, svc)
                       .with_env_variable(f"TT_SERVICE_{name.upper()}", url))
            for name in c.get("hosts", []):
                ctr = (ctr.with_service_binding(name, self._host(f"{owner}-{name}", services))
                       .with_env_variable(f"TT_HOST_{name.upper()}", f"{name}:7000"))
            net_admin = bool(c.get("net_admin"))
            if net_admin:
                ctr = ctr.with_env_variable("TT_NET_ADMIN", "1")
            return await self._run(sem, ctr, ["python3", "-m", "tt", "case", c["id"]], rid, "scenario", net_admin)

        results = list(await asyncio.gather(*(one(c, k) for c in cases for k in range(1, repeat + 1))))
        return await self._fixture_results(get, registry, sorted(needed)) + results

    # -------------------------------------------------------------------- API

    @function
    async def fixtures(self) -> str:
        """Fixture graph: name, tier, number of cases, parents and module."""
        reg = await self._registry()
        users: dict[str, int] = {}
        for c in reg["cases"]:
            users[c["fixture"]] = users.get(c["fixture"], 0) + 1
        lines = [f"{'FIXTURE':24} {'TIER':6} {'CASES':>5}  REQUIRES (MODULE)"]
        for name, f in sorted(reg["fixtures"].items()):
            lines.append(f"{name:24} {'tilr' if f['tilr'] else 'source':6} {users.get(name, 0):>5}  "
                         f"{', '.join(f['requires']) or '-'} ({f['module']})")
        return "\n".join(lines) + "\n"

    @function
    async def fixture(self, name: Annotated[str, Doc("fixture name")]) -> dagger.Directory:
        """One fixture snapshot (its /fx directory), for inspection with export."""
        return await self._fixture_builder(await self._registry())(name)

    @function(cache="never")
    async def test(
        self,
        only: Annotated[str, Doc("comma list of: rust, lint, scenarios (default: all)")] = "",
        filter: Annotated[str, Doc("substring of a test id, e.g. uc_040 or reload")] = "",
        jobs: Annotated[int, Doc("tests running at once")] = 16,
        cached: Annotated[bool, Doc("reuse results of tests whose inputs did not change")] = False,
        repeat: Annotated[int, Doc("run every scenario and Rust test this many times (flakiness hunt)")] = 1,
    ) -> dagger.Directory:
        """Run the tests concurrently; returns report.json, junit.xml, summary.txt, status and logs/."""
        kinds = {k.strip() for k in only.split(",") if k.strip()} or {"rust", "lint", "scenarios"}
        unknown = kinds - {"rust", "lint", "scenarios"}
        if unknown:
            raise ValueError(f"unknown --only value: {', '.join(sorted(unknown))}")
        run_id = "cached" if cached else uuid.uuid4().hex
        sem = asyncio.Semaphore(max(1, jobs))
        started = datetime.datetime.now(datetime.timezone.utc)
        groups = []
        if "rust" in kinds:
            groups.append(self._rust_tests(sem, run_id, filter, max(1, repeat)))
        if "lint" in kinds:
            groups.append(self._lint(sem, run_id, filter))
        if "scenarios" in kinds:
            async def scenarios():
                return await self._scenarios(sem, run_id, filter, await self._registry(), max(1, repeat))
            groups.append(scenarios())
        results: list[Result] = []
        for chunk in await asyncio.gather(*groups, return_exceptions=True):
            if isinstance(chunk, BaseException):
                results.append(Result(f"setup:{type(chunk).__name__}", "setup", "FAIL", 0, _error_text(chunk)))
            else:
                results.extend(chunk)
        return _report(self.runtime(), results, started)


def _failed(get, name: str) -> bool:
    task = get(name)
    return task.done() and task.exception() is not None


def _error_text(e: BaseException) -> str:
    if isinstance(e, dagger.ExecError):
        return f"{e}\n--- stdout\n{e.stdout}\n--- stderr\n{e.stderr}"
    return f"{type(e).__name__}: {e}"


def _safe(rid: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in rid)


UNBUNDLE = ("import json, os\nfor path, text in json.load(open('/bundle.json')).items():\n"
            "    os.makedirs(os.path.dirname(f'/out/{path}') or '/out', exist_ok=True)\n"
            "    open(f'/out/{path}', 'w').write(text)\n")


def _report(runtime: dagger.Container, results: list[Result], started: datetime.datetime) -> dagger.Directory:
    from xml.sax.saxutils import escape, quoteattr

    results.sort(key=lambda r: (r.status == "PASS", r.id))
    counts = {s: sum(r.status == s for r in results) for s in ("PASS", "FAIL", "BLOCKED")}
    elapsed = (datetime.datetime.now(datetime.timezone.utc) - started).total_seconds()
    # One file per test written by a single exec: hundreds of with_new_file layers
    # exceed the overlayfs lower-dir limit when the directory is exported.
    files = {f"logs/{_safe(r.id)}.log": r.log for r in results}
    report = {"started": started.isoformat(), "seconds": round(elapsed, 1), "counts": counts,
              "tests": [{"id": r.id, "kind": r.kind, "status": r.status, "ms": r.ms, "metrics": r.metrics,
                         "log": f"logs/{_safe(r.id)}.log"} for r in results]}
    lines = []
    for r in results:
        if r.status != "PASS":
            tail = "\n".join("      " + l for l in r.log.strip().splitlines()[-15:])
            lines.append(f"{r.status:7} {r.id}  ({r.ms / 1000:.1f}s)\n{tail}")
    measured = [r for r in results if r.metrics]
    if measured:
        lines.append("")
        lines.append("metrics:")
        for r in sorted(measured, key=lambda r: r.id):
            lines.append(f"  {r.id}: " + ", ".join(f"{k}={v}" for k, v in r.metrics.items()))
    by_kind: dict[str, list[int]] = {}
    for r in results:
        k = by_kind.setdefault(r.kind, [0, 0])
        k[0 if r.status == "PASS" else 1] += 1
    slowest = sorted(results, key=lambda r: -r.ms)[:5]
    lines.append("")
    lines.append("slowest: " + ", ".join(f"{r.id} {r.ms / 1000:.1f}s" for r in slowest))
    lines.append("  ".join(f"{k}: {p}/{p + f}" for k, (p, f) in sorted(by_kind.items())))
    lines.append(f"TOTAL {len(results)}: {counts['PASS']} passed, {counts['FAIL']} failed, "
                 f"{counts['BLOCKED']} blocked in {elapsed:.0f}s")
    suites: dict[str, list[tuple[str, Result]]] = {}
    for r in sorted(results, key=lambda r: r.id):
        if r.kind == "rust":
            suite, _, name = r.id[len("rust:"):].partition("::")
            suite = f"rust {suite}"
        elif r.kind == "scenario":
            module, _, name = r.id[len("scenario:"):].partition(":")
            suite = f"scenario {module}"
        else:
            suite, name = r.kind, r.id.split(":", 1)[-1]
        suites.setdefault(suite, []).append((name, r))
    junit = ['<?xml version="1.0" encoding="UTF-8"?>',
             f'<testsuites name="tilr-itests" tests="{len(results)}" failures="{counts["FAIL"]}" '
             f'errors="{counts["BLOCKED"]}" time="{elapsed:.1f}">']
    for suite, members in sorted(suites.items()):
        fails = sum(r.status == "FAIL" for _, r in members)
        errors = sum(r.status == "BLOCKED" for _, r in members)
        seconds = sum(r.ms for _, r in members) / 1000
        junit.append(f'  <testsuite name={quoteattr(suite)} tests="{len(members)}" failures="{fails}" '
                     f'errors="{errors}" time="{seconds:.3f}">')
        for name, r in members:
            junit.append(f'    <testcase classname={quoteattr(suite)} name={quoteattr(name)} time="{r.ms / 1000:.3f}">')
            if r.status == "FAIL":
                junit.append(f'      <failure message="failed">{escape(r.log[-20000:])}</failure>')
            elif r.status == "BLOCKED":
                junit.append(f'      <error message="blocked">{escape(r.log[-20000:])}</error>')
            else:
                junit.append(f'      <system-out>{escape(r.log[-4000:])}</system-out>')
            junit.append("    </testcase>")
        junit.append("  </testsuite>")
    junit.append("</testsuites>")
    files.update({"report.json": json.dumps(report, indent=1) + "\n", "summary.txt": "\n".join(lines) + "\n",
                  "junit.xml": "\n".join(junit) + "\n",
                  "status": "ok\n" if counts["FAIL"] + counts["BLOCKED"] == 0 else "failed\n"})
    return (runtime.with_new_file("/bundle.json", json.dumps(files))
            .with_exec(["python3", "-c", UNBUNDLE])
            .directory("/out"))
