#!/usr/bin/env python3
"""Controlled overlapping linux.sh builds; --docker uses real Docker and rustc."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def mock_docker(args):
    root = Path(os.environ["LINUX_TEST_STATE"])
    if args[0] == "build":
        if os.environ.get("LINUX_TEST_BUILD_FAIL"):
            return 17
        if "--quiet" in args or "-q" in args:
            print("sha256:fixture-image")
        return 0
    assert args[0] == "run", args
    mounts = [args[i + 1].split(":") for i, x in enumerate(args) if x == "-v"]
    source = Path(next(a for a, b in mounts if b == "/src"))
    target_name = next(a for a, b in mounts if b == "/target")
    target = root / target_name
    target.mkdir(exist_ok=True)
    image_index = next(i for i, x in enumerate(args) if x.startswith("sha256:") or x.startswith("tilr-linux-dev"))
    command = args[image_index + 1:]
    (source / "invocation.json").write_text(json.dumps({"args": args, "mounts": mounts, "target": target_name, "image": args[image_index], "command": command}))
    if command == ["cargo", "test", "--workspace", "--features", "tilrd/failpoints", "--locked"]:
        return 0
    command = [x.replace("/src", str(source)).replace("/target", str(target)) for x in command]
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), LINUX_TEST_LABEL=(source / "label").read_text())
    return subprocess.call(command, cwd=source, env=env)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs)


def check(real, script):
    repo = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="linux-script-") as directory:
        root = Path(directory)
        seed = root / "seed"
        seed.mkdir()
        run("git", "init", "-q", str(seed))
        run("git", "-C", str(seed), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture", "--allow-empty")
        worktrees = [root / x / "same-name" for x in ("one", "two")]
        env = dict(os.environ)
        for key in ("TILR_LINUX_PLATFORM", "TILR_LINUX_USER"):
            env.pop(key, None)
        legacy_volume = "tilr-linux-regression-" + root.name
        volumes = {legacy_volume}
        processes = []
        try:
            for tree, label in zip(worktrees, ("A", "B")):
                tree.parent.mkdir()
                run("git", "-C", str(seed), "worktree", "add", "--detach", str(tree), "HEAD")
                (tree / "scripts").mkdir()
                for name in ("linux.sh", "linux-build.sh", "linux.Dockerfile", "linux-entrypoint.sh", "install-tools.sh"):
                    shutil.copy2(script if name == "linux.sh" else repo / "scripts" / name, tree / "scripts" / name)
                # Keep reproduction away from the real shared target used by other workers.
                linux = tree / "scripts/linux.sh"
                linux.write_text(linux.read_text().replace('"tilr-linux-target$suffix"', f'"{legacy_volume}$suffix"'))
                (tree / "label").write_text(label)
                (tree / "Cargo.toml").write_text('[workspace]\nmembers = ["tilrd", "tilr"]\nresolver = "2"\n')
                (tree / "Cargo.lock").write_text('version = 3\n[[package]]\nname = "tilrd"\nversion = "0.1.0"\n[[package]]\nname = "tilr"\nversion = "0.1.0"\n')
                for binary in ("tilrd", "tilr"):
                    (tree / binary / "src").mkdir(parents=True)
                    (tree / binary / "Cargo.toml").write_text(f'[package]\nname = "{binary}"\nversion = "0.1.0"\nedition = "2021"\n')
                    (tree / binary / "src" / "main.rs").write_text(f'fn main() {{ println!("{label}"); }}\n')
                # Distinct source mtimes make Cargo's freshness decision deterministic.
                for path in tree.glob("*/src/main.rs"):
                    os.utime(path, (1 if label == "A" else 4102444800, 1 if label == "A" else 4102444800))
                digest = run("git", "-C", str(tree), "hash-object", "--stdin", input=str(tree.resolve()) + "\n").stdout.strip()
                volumes.add("tilr-linux-target-" + digest)
            if not real:
                tools = root / "bin"
                tools.mkdir()
                docker = tools / "docker"
                docker.write_text(f'#!{sys.executable}\nimport runpy,sys\nsys.argv = [{str(Path(__file__).resolve())!r}, "--mock-docker"] + sys.argv[1:]\nrunpy.run_path(sys.argv[0], run_name="__main__")\n')
                docker.chmod(0o755)
                cargo = tools / "cargo"
                cargo.write_text('''#!/bin/sh
set -eu
profile=debug
case " $* " in *" --release "*) profile=release ;; esac
mkdir -p "$CARGO_TARGET_DIR/$profile"
for binary in tilrd tilr; do
    printf '#!/bin/sh\necho %s\n' "$LINUX_TEST_LABEL" > "$CARGO_TARGET_DIR/$profile/$binary"
    chmod +x "$CARGO_TARGET_DIR/$profile/$binary"
done
''')
                cargo.chmod(0o755)
                env.update(PATH=str(tools) + os.pathsep + env["PATH"], LINUX_TEST_STATE=str(root))
            a, b = worktrees
            command = 'set -eu; cargo build --offline --locked --workspace --bins; mkfifo /target/gate; echo BUILT; read -r token < /target/gate; rm /target/gate; actual=$(/target/debug/tilrd); echo "A observed $actual"; test "$actual" = A'
            with (root / "a.log").open("w+") as error_log:
                first = subprocess.Popen([str(a / "scripts/linux.sh"), "sh", "-c", command], env=env, stdout=subprocess.PIPE, stderr=error_log, text=True)
                processes.append(first)
                while True:
                    line = first.stdout.readline()
                    if line.strip() == "BUILT":
                        break
                    if not line:
                        error_log.seek(0)
                        raise AssertionError("A did not reach its barrier: " + error_log.read())
                try:
                    second = run(str(b / "scripts/linux.sh"), "sh", "-c", 'set -eu; cargo build --offline --locked --workspace --bins; test "$(/target/debug/tilrd)" = B', env=env, timeout=180)
                finally:
                    if real:
                        target = legacy_volume if legacy_volume in (a / "scripts/linux.sh").read_text() else "tilr-linux-target-" + run("git", "-C", str(a), "hash-object", "--stdin", input=str(a.resolve()) + "\n").stdout.strip()
                        container = run("docker", "ps", "-q", "--filter", "volume=" + target).stdout.strip()
                        assert container and "\n" not in container, container
                        run("docker", "exec", container, "sh", "-c", "echo resume > /target/gate", timeout=30)
                    else:
                        target = json.loads((a / "invocation.json").read_text())["target"]
                        # O_RDWR also permits cleanup when the child has already exited.
                        fd = os.open(root / target / "gate", os.O_RDWR)
                        try:
                            os.write(fd, b"resume\n")
                        finally:
                            os.close(fd)
                out, _ = first.communicate(timeout=180)
                assert first.returncode == 0, out
                assert "A observed A" in out, out
            print("overlapping builds: A retained A after B built B")
            if not real:
                def invocation(tree):
                    return json.loads((tree / "invocation.json").read_text())
                original = invocation(a)
                other = invocation(b)
                assert original["target"] != other["target"]
                assert original["image"] == "sha256:fixture-image"
                assert [m for m in original["mounts"] if m[1].endswith("registry")] == [m for m in other["mounts"] if m[1].endswith("registry")]
                alias = root / "alias"
                alias.symlink_to(a, target_is_directory=True)
                run(str(alias / "scripts/linux.sh"), env=env)
                assert invocation(a)["target"] == original["target"]
                platform_env = dict(env, TILR_LINUX_PLATFORM="linux/amd64")
                run(str(a / "scripts/linux.sh"), env=platform_env)
                assert invocation(a)["target"] == original["target"] + "linux-amd64"
                assert "--platform" in invocation(a)["args"]
                before = (a / "invocation.json").read_text()
                failed = subprocess.run([str(a / "scripts/linux.sh")], env=dict(env, LINUX_TEST_BUILD_FAIL="1"), capture_output=True)
                assert failed.returncode != 0
                assert (a / "invocation.json").read_text() == before
            run(str(a / "scripts/linux-build.sh"), env=env, timeout=180)
            for binary in ("tilrd", "tilr"):
                artifact = next((a / "dist").glob(f"linux-*/{binary}"))
                if real:
                    # Linux binaries cannot be executed on the host; compare in-container.
                    run(str(a / "scripts/linux.sh"), "sh", "-c", f'test "$(/src/dist/linux-$(uname -m)/{binary})" = A', env=env, timeout=180)
                else:
                    assert run(str(artifact)).stdout.strip() == "A"
            print("target reuse, platform isolation, release copy: ok")
        finally:
            for process in processes:
                if process.poll() is None:
                    process.kill()
                    process.wait()
            if real:
                # Remove only this fixture's hash-named target volumes.
                for volume in volumes:
                    subprocess.run(["docker", "volume", "rm", volume], capture_output=True)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--mock-docker":
        sys.exit(mock_docker(sys.argv[2:]))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--docker", action="store_true")
    parser.add_argument("--linux-script", type=Path, default=Path(__file__).with_name("linux.sh"))
    options = parser.parse_args()
    check(options.docker, options.linux_script.resolve())
