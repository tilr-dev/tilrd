# Working in this repo

tilrd: the tilr terminal server and its client. A Rust workspace: `tilr-proto` (wire
protocol), `tilr-screen` (screen model on libghostty-vt), `tilrd` (the server binary
`tilrd`), `tilr` (the client binary `tilr`, which starts tilrd when none runs) and
`tilr-driver` (a scripted client for the black-box tests in `itests/`). README.md describes
the server, the client, the protocol and the build; USE_CASES.md lists what the tests
cover, itests/USE_CASES.md what the black-box tests cover. Repo-specific notes go below
the line.

## Before every commit

- **The checks CI runs must be green** (`.github/workflows/ci.yml`); green here, green there:

  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo clippy --workspace --all-targets --features tilrd/failpoints --locked -- -D warnings
  cargo nextest run --workspace --features tilrd/failpoints --locked
  cargo deny check
  gitleaks git --redact --no-banner --exit-code 1 .
  ```

  CI also checks the MSRV (`rust-version` in Cargo.toml, `cargo +1.90 check --workspace
  --all-targets --features tilrd/failpoints --locked`) and runs the tests on Linux and macOS. `scripts/linux.sh` runs
  the suite in a Linux container from another OS (`scripts/linux.sh cargo clippy ...` the
  lint), as a non-root user with a passwd entry and a home directory, like CI's `runner`
  (`scripts/linux-entrypoint.sh`): root skips file permission checks and has another login
  shell setup, so a run as root does not show what CI will. `TILR_LINUX_USER=root` runs as
  root when that is what you want.
- `cargo fmt --all` fixes formatting. Clippy findings are fixed, not silenced; an `#[allow]`
  names the lint and says why.
- `Cargo.lock` is committed; every cargo command in CI runs with `--locked`.
- Do not bypass hooks or checks with `--no-verify`.

## Build prerequisites

- Rust (stable; MSRV in `rust-version`), [Zig](https://ziglang.org) 0.15.2 (libghostty-vt-sys
  builds Ghostty with it) and `protoc` (tilr-proto's build.rs). `scripts/install-tools.sh DIR`
  installs the pinned Zig, protoc and cargo-nextest (which runs the tests), as CI does.
- Without a shared package, the first build needs network access: libghostty-vt-sys clones
  Ghostty at a pinned commit. `scripts/build-ghostty-vt.sh` builds a shared package once;
  set PKG_CONFIG_PATH to its share/pkgconfig directory (README, "Build").
- macOS (checked 2026-10-19): Zig 0.15.2 fails with the macOS 27.0 SDK on this
  Mac; the tracked `.cargo/config.toml` selects CLT MacOSX15.4.sdk. Check it with
  `DEVELOPER_DIR=/Library/Developer/CommandLineTools xcrun --sdk macosx15.4 --show-sdk-path`.
  After a Zig upgrade passes a clean build with the current SDK, remove the SDK
  pin and installer preflight; see README ("Build").

## Commits

- **Conventional Commits**: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `ci`, `perf`,
  optional scope, `!` / `BREAKING CHANGE:` for a break, footers `Refs: #N` / `Closes: #N`.
- **No AI attribution**: no "Generated with …" lines, no agent `Co-Authored-By`.
- One logical change per commit; short branches (`feat/12-short-name`), squash-merged into
  `main`, which stays green.

## Releases

- release-please (`.github/workflows/release.yml`): `feat` / `fix` commits open or update the
  release PR; merging it tags `vX.Y.Z`, builds the binaries for the GitHub release and
  publishes the Linux ones to the CDN (`scripts/release.sh --upload-only`).
- `scripts/release.sh --publish` / `--upload-only` need `CLOUDFLARE_ACCOUNT_ID` exported (by
  hand e.g. via `infisical run --path /cloudflare`; in CI the `cdn` environment's variable)
  besides wrangler credentials; the script fails without it.
- Do not edit versions (`[workspace.package] version`, Cargo.lock) or CHANGELOG.md by hand.
- The `aarch64-unknown-linux-gnu` release build (on `ubuntu-24.04-arm`, free only for public
  repos) is off by default: set repo variable RELEASE_LINUX_ARM64=true once the repo is
  public to enable the arm64 build. While it is off, the release and the CDN get
  `linux-x86_64` only and `releases/linux-aarch64/latest` is left as it is.
- The CDN layout (`releases/<platform>/latest`,
  `releases/<version>/tilr-<version>-<platform>.tar.gz` holding `tilrd` and `tilr`, with its
  `.sha256`) is what `tilrd update` (`crates/tilrd/src/update.rs`, run by `tilr update`) and
  the installer (`scripts/install.sh`, which `tilr.sh` serves and the black-box tests run) read: do
  not change one without the others.

## Secrets

- Assume the repo is public: no tokens, keys, `.env` files or personal data in code, tests,
  fixtures or commit messages. gitleaks scans the history in CI.

---

## This repo

- One supervisor process routes packets between clients and pane processes (one per pane,
  owning the pty, the shell and the screen model); a reload execs the new executable in place
  and hands state over through files in the runtime directory.
- Tests in `crates/tilrd/tests/` (and `crates/tilr/tests/` for the client) start the real server and drive it as a client; the
  shared harness is `tests/common/mod.rs`. A test's name states its use case; add the use case
  to USE_CASES.md with the test.
- `failpoints` (`crates/tilrd/src/fail.rs`) is a test-only feature; a release is built
  without it.
- The protocol schema is `crates/tilr-proto/proto/tilr.proto`; clients copy it from here
  (the `server` plugin's `npm run proto`, the test client), so a change there is a protocol
  change. Before the first public release, keep `PROTOCOL_VERSION` at 1 for all changes.
  Afterward, bump only for incompatibility with a client or server of the previous public
  release, once per release as a separate step in its release milestone. Feature flags
  name only capabilities a server can be run without; see README "Protocol".
