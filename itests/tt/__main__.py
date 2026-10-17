"""python3 -m tt list | fixture MODULE:NAME | case MODULE_SHORT:FUNCTION"""

import importlib
import json
import pkgutil
import shutil
import sys
import traceback
from pathlib import Path

import tt

ROOT = Path(__file__).resolve().parents[1]


def import_all():
    for package in ("fixtures", "scenarios"):
        for info in sorted(pkgutil.iter_modules([str(ROOT / package)]), key=lambda i: i.name):
            importlib.import_module(f"{package}.{info.name}")


def registry() -> dict:
    import_all()
    fixtures = tt.FIXTURES
    errors = []
    for f in fixtures.values():
        errors += [f"fixture {f.name}: unknown parent {p}" for p in f.requires if p not in fixtures]
        if not f.tilr and any(fixtures[p].tilr for p in f.requires if p in fixtures):
            errors.append(f"fixture {f.name}: source tier cannot build on a tilr-tier parent")
        if f.check is None:
            errors.append(f"fixture {f.name}: no @{f.name}.check — every fixture verifies its state")
    for c in tt.CASES:
        if c.fixture and c.fixture not in fixtures:
            errors.append(f"case {c.id}: unknown fixture {c.fixture}")

    def visit(name, path):
        if name in path:
            errors.append("fixture cycle: " + " -> ".join(path + [name]))
            return
        for p in fixtures[name].requires if name in fixtures else []:
            visit(p, path + [name])
    for name in fixtures:
        visit(name, [])
    ids = [c.id for c in tt.CASES]
    errors += [f"duplicate case {i}" for i in sorted({i for i in ids if ids.count(i) > 1})]
    if errors:
        sys.exit("tt registry errors:\n  " + "\n  ".join(errors))
    return {
        "fixtures": {f.name: {"module": f.module, "requires": f.requires, "tilr": f.tilr, "doc": f.doc}
                     for f in fixtures.values()},
        "cases": [{"id": c.id, "module": c.module, "fixture": c.fixture, "services": c.services,
                   "timeout": c.timeout, "title": c.title, "hosts": c.hosts, "net_admin": c.net_admin}
                  for c in tt.CASES],
    }


def materialize(root: Path):
    """Copy the snapshot into the container's writable layer: files of a lower overlayfs
    layer cannot be renamed (EXDEV) or hard-linked across layers."""
    if not root.exists() or not any(root.iterdir()):
        return
    copy = root.with_name(root.name + ".tt-copy")
    shutil.copytree(root, copy, symlinks=True)
    for child in list(root.iterdir()):
        shutil.rmtree(child) if child.is_dir() and not child.is_symlink() else child.unlink()
    for child in list(copy.iterdir()):
        child.rename(root / child.name)
    copy.rmdir()


def run(label: str, fn, t: tt.T) -> int:
    print(f"== {label}  (backend {t.backend})")
    try:
        fn(t)
        if t.soft_failures:
            raise tt.Failure(f"{len(t.soft_failures)} check(s) failed:\n  " + "\n  ".join(t.soft_failures))
    except tt.Blocked as e:
        print(f"\nBLOCKED {label}: {e}")
        return 77
    except Exception as e:
        print(f"\nFAILED {label}: {e}")
        traceback.print_exc(file=sys.stdout)
        t.dump_logs()
        return 1
    finally:
        t.cleanup()
    print(f"\nPASS {label}")
    t.dump_logs()
    return 0


def main(argv: list[str]) -> int:
    sys.stdout.reconfigure(line_buffering=True)
    if argv[:1] == ["list"]:
        print(json.dumps(registry(), indent=1))
        return 0
    if len(argv) == 2 and argv[0] == "fixture":
        module, name = argv[1].split(":")
        importlib.import_module(module)
        f = tt.FIXTURES[name]
        t = tt.T()
        t.root.mkdir(exist_ok=True)
        materialize(t.root)
        code = run(f"fixture {name}: build", f.build, t)
        if code == 0:
            code = run(f"fixture {name}: check", f.check, tt.T())
        return code
    if len(argv) == 2 and argv[0] == "case":
        short, _ = argv[1].split(":")
        importlib.import_module(f"scenarios.{short}")
        c = next(c for c in tt.CASES if c.id == argv[1])
        t = tt.T()
        t.root.mkdir(exist_ok=True)
        materialize(t.root)
        return run(f"{c.id} {c.title}".strip(), c.fn, t)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
