# Linux build and test environment for tilr. Used by scripts/linux.sh (and through it by
# scripts/linux-build.sh, scripts/release.sh and .github/workflows/release.yml).
# The base image is pinned by digest (Rust 1.99 on Debian 12: release binaries need glibc
# 2.36 or newer); Dependabot bumps it (docker ecosystem, /scripts).
FROM rust:1.99-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler git xz-utils unzip ca-certificates curl procps \
    && rm -rf /var/lib/apt/lists/*

# clippy, so the lint CI runs on Linux can run here too.
RUN rustup component add clippy

# libghostty-vt is built with Zig and the tests run with cargo-nextest: the pinned
# versions, checked against their SHA-256 sums.
COPY install-tools.sh /usr/local/lib/tilr/install-tools.sh
RUN /usr/local/lib/tilr/install-tools.sh /opt/tilr-tools zig nextest \
    && ln -s /opt/tilr-tools/zig /usr/local/bin/zig \
    && ln -s /opt/tilr-tools/cargo-nextest /usr/local/bin/cargo-nextest

# Prefetch GitHub dependencies of libghostty-vt-sys 0.2.2's pinned Ghostty via
# codeload, checking the exact build.zig.zon hashes. GitHub's redirect headers
# exceed Zig 0.15.2's HTTP buffer (README, Build); codeload serves the same archives.
# Keep these hashes in sync with Ghostty's dcimgui and vaxis dependencies.
ENV ZIG_GLOBAL_CACHE_DIR=/opt/tilr-tools/zig-cache
RUN test "$(zig fetch https://codeload.github.com/ocornut/imgui/tar.gz/refs/tags/v1.92.5-docking)" = "N-V-__8AAEbOfQBnvcFcCX2W5z7tDaN8vaNZGamEQtNOe0UI" \
    && test "$(zig fetch https://codeload.github.com/ivanstepanovftw/zigimg/tar.gz/d7b7ab0ba0899643831ef042bd73289510b39906)" = "zigimg-0.1.0-8_eo2vHnEwCIVW34Q14Ec-xUlzIoVg86-7FU2ypPtxms" \
    && test "$(zig fetch https://codeload.github.com/jacobsandlund/uucode/tar.gz/5f05f8f83a75caea201f12cc8ea32a2d82ea9732)" = "uucode-0.1.0-ZZjBPj96QADXyt5sqwBJUnhaDYs_qBeeKijZvlRa0eqM" \
    && chmod -R a+rwX "$ZIG_GLOBAL_CACHE_DIR"

# Commands run as a non-root user with a passwd entry, as on the GitHub runner.
COPY linux-entrypoint.sh /usr/local/lib/tilr/linux-entrypoint.sh
ENTRYPOINT ["/usr/local/lib/tilr/linux-entrypoint.sh"]

ENV CARGO_TARGET_DIR=/target
WORKDIR /src
