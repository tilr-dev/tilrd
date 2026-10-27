#!/usr/bin/env python3
"""Exercise the shared build script with deterministic external tool fixtures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="ghostty fixture ") as directory:
    base = Path(directory)
    tools = base / "bin"
    tools.mkdir()
    sdk = base / "MacOSX15.4.sdk"
    sdk.mkdir()
    events = base / "events"
    fixtures = {
        "uname": 'case "$1" in -s) echo "${MOCK_HOST:-Darwin}" ;; -m) echo arm64 ;; esac\n',
        "git": '''if [ "$1" = hash-object ]; then exec python3 -c 'import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())'; fi
printf 'git %s\\n' "$*" >> "$EVENTS"
if [ "$1" = init ]; then mkdir -p "$3"; fi
''',
        "zig": '''if [ "$1" = version ]; then echo "${MOCK_ZIG:-0.15.2}"; exit; fi
printf 'zig %s SDK=%s DEVELOPER=%s\\n' "$*" "${SDKROOT:-}" "${DEVELOPER_DIR:-}" >> "$EVENTS"
[ "${MOCK_FAIL:-0}" = 0 ] || exit 23
if [ -n "${READY_FIFO:-}" ]; then
    printf 'ready\\n' > "$READY_FIFO"
    read -r release < "$RELEASE_FIFO"
fi
while [ "$1" != --prefix ]; do shift; done
shift
mkdir -p "$1/lib" "$1/share/pkgconfig"
touch "$1/lib/libghostty-vt.a"
printf 'prefix=%s\\nlibdir=${prefix}/lib\\nName: libghostty-vt-static\\nDescription: fixture\\nVersion: 0.1.0\\nLibs: ${libdir}/libghostty-vt.a\\n' "$1" > "$1/share/pkgconfig/libghostty-vt-static.pc"
''',
    }
    for name, body in fixtures.items():
        path = tools / name
        path.write_text("#!/bin/sh\nset -eu\n" + body)
        path.chmod(0o755)
    env = dict(os.environ, PATH=f"{tools}:{os.environ['PATH']}",
               TILR_GHOSTTY_VT_CACHE=str(base / "cache"), EVENTS=str(events),
               SDKROOT=str(sdk), DEVELOPER_DIR="/fixture/developer")

    def run(**overrides):
        return subprocess.run([str(ROOT / "scripts/build-ghostty-vt.sh")],
                              env=dict(env, **overrides), text=True, capture_output=True)

    first = run()
    assert first.returncode == 0, first.stderr
    prefix = Path(first.stdout.strip())
    assert (prefix / ".complete").is_file()
    pc = prefix / "share/pkgconfig/libghostty-vt-static.pc"
    assert pc.read_text().startswith("prefix=${pcfiledir}/../..\n")
    log = events.read_text()
    assert "fetch --depth 1 origin a887df42c56f6de86c0fe6da9c4eeca37931e083" in log
    for flag in ("-Demit-lib-vt=true", "-Doptimize=ReleaseFast", "-Dcpu=baseline",
                 "-Demit-xcframework=false", "-Dapp-runtime=none"):
        assert flag in log
    assert f"SDK={sdk} DEVELOPER=/fixture/developer" in log
    again = run()
    assert again.returncode == 0 and again.stdout == first.stdout
    assert events.read_text() == log, "cached invocation cloned or built again"
    # An exited child supplies a known dead PID without timing or PID guesses.
    child = subprocess.Popen(["/bin/sh", "-c", "exit 0"])
    child.wait()
    lock = Path(f"{prefix}.lock")
    leftover = Path(f"{prefix}.tmp.abandoned")
    lock.mkdir()
    (lock / "pid").write_text(f"{child.pid}\n")
    leftover.mkdir()
    (leftover / "checkout").touch()
    recovered_published = run()
    assert recovered_published.returncode == 0, recovered_published.stderr
    assert recovered_published.stdout == first.stdout and events.read_text() == log
    assert not lock.exists() and not leftover.exists()
    shutil.rmtree(prefix)
    prefix.mkdir()
    (prefix / "unfinished").touch()
    lock.mkdir()
    (lock / "pid").write_text(f"{child.pid}\n")
    leftover.mkdir()
    (leftover / "checkout").touch()
    recovered_stale = run()
    assert recovered_stale.returncode == 0, recovered_stale.stderr
    assert recovered_stale.stdout == first.stdout
    assert pc.is_file() and (prefix / "lib/libghostty-vt.a").is_file()
    assert not (prefix / "unfinished").exists()
    assert not lock.exists() and not leftover.exists()
    shutil.rmtree(prefix)
    lock.mkdir()
    (lock / "pid").write_text(f"{os.getpid()}\n")
    leftover.mkdir()
    log = events.read_text()
    active = run()
    assert active.returncode != 0 and "Build already in progress" in active.stderr
    assert events.read_text() == log
    assert (lock / "pid").read_text() == f"{os.getpid()}\n"
    assert leftover.is_dir()
    shutil.rmtree(lock)
    shutil.rmtree(leftover)
    # Recovery also handles death before the owner PID was written.
    lock.mkdir()
    recovered_empty = run()
    assert recovered_empty.returncode == 0, recovered_empty.stderr
    assert not lock.exists()
    shutil.rmtree(prefix)
    lock.mkdir()
    (lock / "pid").write_text(f"{child.pid}\n")
    leftover.mkdir()
    ready, release = base / "ready", base / "release"
    os.mkfifo(ready)
    os.mkfifo(release)
    builder = subprocess.Popen([str(ROOT / "scripts/build-ghostty-vt.sh")],
                               env=dict(env, READY_FIFO=str(ready), RELEASE_FIFO=str(release)),
                               text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        with ready.open() as handshake:
            assert handshake.readline() == "ready\n"
        owner = (lock / "pid").read_text()
        active_work = list(prefix.parent.glob(f"{prefix.name}.tmp.*"))
        assert active_work and not leftover.exists()
        log = events.read_text()
        contender = run()
        assert contender.returncode != 0 and "Build already in progress" in contender.stderr
        assert events.read_text() == log and (lock / "pid").read_text() == owner
        assert all(path.is_dir() for path in active_work)
        # The guard must protect recovery even with stale owner metadata.
        (lock / "pid").write_text(f"{child.pid}\n")
        stale_contender = run()
        assert stale_contender.returncode != 0, stale_contender.stderr
        assert events.read_text() == log
        assert all(path.is_dir() for path in active_work)
        (lock / "pid").write_text(owner)
    finally:
        with release.open("w") as handshake:
            handshake.write("continue\n")
        stdout, stderr = builder.communicate()
    assert builder.returncode == 0, stderr
    assert stdout == first.stdout and pc.is_file()
    assert not lock.exists() and not list(prefix.parent.glob(f"{prefix.name}.tmp.*"))
    changed = run(MOCK_ZIG="0.16.0")
    assert changed.returncode == 0 and changed.stdout != first.stdout
    other_sdk = base / "other.sdk"
    other_sdk.mkdir()
    changed_sdk = run(SDKROOT=str(other_sdk))
    assert changed_sdk.returncode == 0 and changed_sdk.stdout != first.stdout
    failed = run(MOCK_ZIG="failure", MOCK_FAIL="1")
    assert failed.returncode != 0
    assert not list((base / "cache").rglob("*.lock"))
    assert not list((base / "cache").rglob("*.tmp.*"))
    recovered = run(MOCK_ZIG="failure")
    assert recovered.returncode == 0, recovered.stderr
    linux = run(MOCK_HOST="Linux", SDKROOT="/absent")
    assert linux.returncode == 0 and linux.stdout != first.stdout
    missing = run(SDKROOT="/absent")
    assert missing.returncode != 0 and "Missing SDK" in missing.stderr
print("shared Ghostty build fixture passed")
