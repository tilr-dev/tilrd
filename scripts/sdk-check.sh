#!/bin/sh
# The checks of the bus SDKs (sdk/deno, sdk/python), as CI's `sdk` job runs them:
#
#   scripts/sdk-check.sh
#
# 1. scripts/gen-bus-sdk.sh --check: the generated schema code matches crates/tilr-bus/proto/;
# 2. sdk/deno: deno fmt --check, deno lint, deno check, that nothing the library runs
#    builds code at run time (`eval(`, `Function(`: its modules, its generated code and
#    the npm packages they reach; a page with a strict Content-Security-Policy could not
#    run it), deno test -A;
# 3. sdk/python: ruff format --check, ruff check, mypy, pytest (through uv run, uv.lock
#    frozen).
#
# The tests start the tilrd under test (`TILRD`, else target/debug/tilrd: build it first
# with `cargo build -p tilrd --locked`) in temporary directories, and each library's test
# runtime; the Python tests also run the Deno runtime (cross-language), so both run here.
# Needs deno, uv and protoc 36.2 on PATH (scripts/install-tools.sh DIR deno uv protoc).
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
tilrd=${TILRD:-$root/target/debug/tilrd}
[ -x "$tilrd" ] || { echo "sdk-check: no tilrd at $tilrd: cargo build -p tilrd --locked (or set TILRD)" >&2; exit 1; }
export TILRD="$tilrd"

step() { printf '\n== %s\n' "$*"; }

step "generated schema code"
"$root/scripts/gen-bus-sdk.sh" --check

step "sdk/deno"
cd "$root/sdk/deno"
deno fmt --check
deno lint
deno check --frozen mod.ts test/ tools/
deno run --quiet --frozen --allow-run --allow-read tools/check_no_eval.ts
deno test --frozen -A test/

step "sdk/python"
cd "$root/sdk/python"
uv run --frozen ruff format --check
uv run --frozen ruff check
uv run --frozen mypy
uv run --frozen pytest -q

printf '\nsdk-check: all passed\n'
