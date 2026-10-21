# Design

## Decisions

### D1. One command everywhere, no retries
Every place that names the test command uses exactly `cargo nextest run --workspace --features tilrd/failpoints --locked`: AGENTS.md, README, `scripts/linux.sh`, CI. No `--retries` and no configured retries: a failure is reported as it happened; a flake is filed with its cause and fixed, not retried away. A static test checks that the four places carry the command and none still names `cargo test --workspace`.

### D2. No separate doctest step
nextest does not run doctests. The workspace has none: the code fences in doc comments are `json`, `toml` or `text` blocks, and no crate includes a Markdown file as docs (`doc = include_str!`). So no `cargo test --doc` step is added; one is needed only once a Rust doctest appears.

### D3. Installed as a pinned release archive
cargo-nextest comes from its GitHub release (`cargo-nextest-<version>-<target>.tar.gz`; targets `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `universal-apple-darwin` for both macOS architectures), checked against SHA-256 sums pinned in `scripts/install-tools.sh`, as Zig and protoc are. The archive holds the single `cargo-nextest` binary; it is extracted into `DIR/cargo-nextest-<version>/` and linked as `DIR/cargo-nextest`, which `cargo nextest` finds on PATH.
- Rejected: `cargo install cargo-nextest --locked`: builds from source on every CI run and container build (minutes), and pins nothing beyond the version.

### D4. No nextest configuration
The defaults (one process per test, as many test threads as CPUs) are what the suite needs; no `.config/nextest.toml`.
