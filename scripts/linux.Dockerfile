# Linux build and test environment for tilr. Used by scripts/linux.sh (and through it by
# scripts/linux-build.sh, scripts/release.sh and .github/workflows/release.yml).
# The base image is pinned by digest (Rust 1.99 on Debian 12: release binaries need glibc
# 2.36 or newer); Dependabot bumps it (docker ecosystem, /scripts).
FROM rust:1.99-bookworm@sha256:e12566d5abc17a0da741628782cdb504b68b8961f5bfcf677936bf3e8b32de29

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler git xz-utils unzip ca-certificates curl procps \
    && rm -rf /var/lib/apt/lists/*

# libghostty-vt is built with Zig: the pinned version, checked against its SHA-256.
COPY install-tools.sh /usr/local/lib/tilr/install-tools.sh
RUN /usr/local/lib/tilr/install-tools.sh /opt/tilr-tools zig \
    && ln -s /opt/tilr-tools/zig /usr/local/bin/zig

ENV CARGO_TARGET_DIR=/target
WORKDIR /src
