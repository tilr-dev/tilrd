# Ghostty VT patches

`libghostty-vt-0.2.2` and `libghostty-vt-sys-0.2.2` come from the crates.io
0.2.2 archives, upstream https://github.com/uzaaft/libghostty-rs at
c9876b2902d5bb3e1f3d61b6395059f1e5e7b721. Their upstream source files are
retained with local changes for `Terminal::erase`, its FFI declaration, and native
build/discovery. The workspace patches crates.io to these directories.

`ghostty/direct-erase.patch` applies to ghostty-org/ghostty commit
a887df42c56f6de86c0fe6da9c4eeca37931e083 (MIT licensed). It adds a C/Zig
host erase operation that bypasses the program parser and the ED prompt heuristic.
Both native build paths apply this file. Source cache stamps include its entire
content and shared prefixes hash its content. The patched package name
`libghostty-vt-static-tilr-erase-v1` prevents linking an unpatched installation.
When changing the C ABI, advance that identity in both builders.

To refresh, download the exact crates.io archives, retain licenses, reapply the
Rust/build changes, rebase the Ghostty patch against the new pinned commit, and
run the screen and shared-build tests on both platforms. Source overrides must
already include the patch; external checkouts are never modified by the builder.
