# Linux build and test environment for tilr. Used by scripts/linux.sh.
FROM rust:1-bookworm

ARG ZIG_VERSION=0.15.2

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler git xz-utils ca-certificates curl procps \
    && rm -rf /var/lib/apt/lists/*

# libghostty-vt is built with Zig.
RUN curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-$(uname -m)-linux-${ZIG_VERSION}.tar.xz" \
    | tar -xJ -C /opt \
    && ln -s "/opt/zig-$(uname -m)-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig

ENV CARGO_TARGET_DIR=/target
WORKDIR /src
