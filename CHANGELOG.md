# Changelog

## [0.2.0](https://github.com/tilr-dev/tilrd/compare/v0.1.0...v0.2.0) (2026-10-26)


### ⚠ BREAKING CHANGES

* count connections closed without Hello and rename metrics to tilrd_
* a v2 client is refused with "unsupported protocol version 2: this server speaks 3", and a v3 client by a v2 server. tilr, tilr-driver, the Tilr app's server plugin and tilr-mac update together with the server. "ping" is no longer in HelloAck.features: a client that checked for it sends Ping unconditionally.
* **service:** `tilr service install` over a hand-written tilr.service or plist now fails instead of replacing it; move the file away (or add the `tilr service` mark) and install again.
* **reload:** a server no longer reloads into an executable that does not print reload-check 1 (any release before this one); a rollback to such a release is installed but needs a restart of the server to run.
* **tilr:** the token from the state directory is sent only to loopback --connect / TILR_CONNECT targets (localhost, 127.0.0.0/8, ::1). For another host, give its token with --token, TILR_TOKEN or token in client.toml.
* **update:** the CDN no longer has tilrd-<platform> or tilr-server-<platform>; a server from before this change (tilr-server, or a tilrd that asks for the loose file) cannot update itself to a new release and has to be reinstalled once.

### Features

* **cloud:** make https://wss.tilr.dev the default tilr Cloud URL ([24277c9](https://github.com/tilr-dev/tilrd/commit/24277c9e6b211b2f768f0cf3762ee28f4880d07e))
* copy from a pane to the client's clipboard ([3f71339](https://github.com/tilr-dev/tilrd/commit/3f713392f128095b91c753194e867ca408a2ba77))
* count connections closed without Hello and rename metrics to tilrd_ ([6fe28c2](https://github.com/tilr-dev/tilrd/commit/6fe28c2d43378fa2f79ec0c61c77041a781eb5da))
* **failpoints:** add a pause action that holds a point until the test releases it ([d059569](https://github.com/tilr-dev/tilrd/commit/d0595696c951db4b1380959e3de10286d9d941b2))
* **itests:** add a stand that occupies the default port for one case ([63047c1](https://github.com/tilr-dev/tilrd/commit/63047c16332c676bf48aa1dc38f4b31b357b852c))
* **panes:** show the shell, cwd and foreground command in the pane process title ([d9dd361](https://github.com/tilr-dev/tilrd/commit/d9dd3616b39463364de5ed3879ac3dd588aad616))
* protocol version 3 is the mandatory wire contract ([3dc42e2](https://github.com/tilr-dev/tilrd/commit/3dc42e2750431099495d4ff151b37229e69a57b6))
* **release:** publish scripts/install.sh to the CDN after every latest ([aaecaf4](https://github.com/tilr-dev/tilrd/commit/aaecaf4be6e2b3f23bc5defa3c9b36609a3006c0))
* **tilrd:** let tests control measured time and the SOCKS deadline ([6d32977](https://github.com/tilr-dev/tilrd/commit/6d32977c4e4423f1e8dfa5a5b51fc46be8b1760a))
* **tilrd:** push PaneActivity to clients on bell and finished command ([6b85c44](https://github.com/tilr-dev/tilrd/commit/6b85c44b954512d7592584df4651ed878e21b777))
* **tilrd:** take the log filter from RUST_LOG ([81f90bb](https://github.com/tilr-dev/tilrd/commit/81f90bba96e8e52e5382d573ab279c8016d6ba48))
* **update:** ship tilrd and tilr as one release archive ([2a8b425](https://github.com/tilr-dev/tilrd/commit/2a8b425e31db190454f81c91be453cb122f00c2e))


### Bug Fixes

* **agent:** keep each server's agent sockets in a directory of its own ([5e0f703](https://github.com/tilr-dev/tilrd/commit/5e0f703b690c08d7844a75228c771183292da3db))
* Alt sends ESC and the character the key produced ([92e4f43](https://github.com/tilr-dev/tilrd/commit/92e4f43b8d2d3267db490b075c5275dab38f5a61))
* build on macOS from a clean target by pinning the 15.4 SDK ([e190558](https://github.com/tilr-dev/tilrd/commit/e190558c03297b191dc08c90ff3381db6a5913b2))
* **build:** watch the real git dir so worktrees stop rebuilding tilr and tilrd ([cb006dc](https://github.com/tilr-dev/tilrd/commit/cb006dc957524abb4895b33ebe2c7f2a141c423f))
* close a connection that says no Hello within 10 s ([f32643d](https://github.com/tilr-dev/tilrd/commit/f32643da9cbbb0735de1ac0b00e4487a79e0c55b))
* **cloud:** never leave cloudflared running after the server ([0aff8e0](https://github.com/tilr-dev/tilrd/commit/0aff8e06d49f3ca6d38f38821a0533f28874c00a))
* **cloud:** reap cloudflared on stop so a reload leaves no zombie ([7210b2c](https://github.com/tilr-dev/tilrd/commit/7210b2cf1fc787c959586538d986dc6162fdb550))
* end what ignores SIGHUP in a pane when the server stops ([47bce42](https://github.com/tilr-dev/tilrd/commit/47bce42ab22d32a706382352acf35c13d2416b06))
* **exec:** always end an exec, last with ExecExit, never losing a reply ([48efd35](https://github.com/tilr-dev/tilrd/commit/48efd35cc8140793bd9d70ce836fda3c13b83f02))
* handle a signal that arrives while the server starts ([f8fb63b](https://github.com/tilr-dev/tilrd/commit/f8fb63b5e8a5616979bfa3309d90d1003fa1e290))
* **host:** give each of one user's servers its own host socket ([ddcc453](https://github.com/tilr-dev/tilrd/commit/ddcc453fbd12ea04c68e869e09bf896c01d7ec82))
* **host:** send nothing to a host socket another user serves ([248dd56](https://github.com/tilr-dev/tilrd/commit/248dd56ee080a9cbc120ae770064946cfb61cdf5))
* **itests:** keep the last run in coverage when there is no local report ([d2b6e50](https://github.com/tilr-dev/tilrd/commit/d2b6e50972faa5de52f21147c2b7558c2ea09d5d))
* **itests:** ship root tests/ to the Dagger build so scenario test binaries compile ([60e3e50](https://github.com/tilr-dev/tilrd/commit/60e3e501f977f972b4ab832b9aa6493b9e167d6f))
* keep the bulk window moving and bring a lagging client its last screen ([7d18ae3](https://github.com/tilr-dev/tilrd/commit/7d18ae3237bf35567ad10073919b6208efd4c5d7))
* keep the service manager's variables out of panes and execs ([4e7812b](https://github.com/tilr-dev/tilrd/commit/4e7812b46e6b4f438021c9aa53fe430917a97813))
* keep the shared runtime files usable while another server starts ([72b0422](https://github.com/tilr-dev/tilrd/commit/72b042200436e4c0db8589e4395d3b8a07719d69))
* **linux:** give each worktree its own cargo target volume in scripts/linux.sh ([6bdfcd4](https://github.com/tilr-dev/tilrd/commit/6bdfcd48bb9092344fd7124f99745796b8f7d6b1))
* **pane:** keep the pane tools first on PATH after the login profile ([0590786](https://github.com/tilr-dev/tilrd/commit/059078650955786e6c9cc00e7eabb529a8ce569d))
* pass the Linux CI checks ([#5](https://github.com/tilr-dev/tilrd/issues/5)) ([417a7b9](https://github.com/tilr-dev/tilrd/commit/417a7b9ed8eb43339b90b6accb6ba3f455e36c90))
* **reload:** carry wraps, images in the history and the close deadline ([53e7ff2](https://github.com/tilr-dev/tilrd/commit/53e7ff2c08460a36552d3193701cdfde6fed660b))
* **reload:** exec only into a successor that showed it can resume ([4298b15](https://github.com/tilr-dev/tilrd/commit/4298b15f5f72c600926031f9f36fb0f3dfc63cf9))
* **reload:** hold signals across the exec; keep the socket on a refusal ([a0f1ae9](https://github.com/tilr-dev/tilrd/commit/a0f1ae9a10920192184c7fdf4f7f7259c4dad001))
* **reload:** keep a stop and SIGPIPE right around the exec ([c6deafc](https://github.com/tilr-dev/tilrd/commit/c6deafc2140316b8f9122f8175631d8d48d4eb66))
* **reload:** let no thread take a held signal while the supervisor execs ([060c7e4](https://github.com/tilr-dev/tilrd/commit/060c7e4f5db9b7050119339d19be995fda3b1b14))
* **screen:** keep the cursor and margins under origin mode across a reload ([af85db3](https://github.com/tilr-dev/tilrd/commit/af85db35ded27c09a5717c99ed4654cbaff73d7e))
* send a new subscriber the full frame before any delta ([1ff5555](https://github.com/tilr-dev/tilrd/commit/1ff555557853c5c390a6d9fbda40a215bc8be8ba))
* **service:** never replace a unit or plist install did not write ([1d44e4b](https://github.com/tilr-dev/tilrd/commit/1d44e4b3d72910d796bd1c949b33d8f6e5ffd73c))
* sync finished file writes and installed releases to disk ([42a4cc2](https://github.com/tilr-dev/tilrd/commit/42a4cc28729e775de9743c01f59fd7bdb9e80792))
* **test:** keep test server cleanup working when a log wait fails ([b62c76b](https://github.com/tilr-dev/tilrd/commit/b62c76b0269dfaf283d1a84ec7e3edccae374cf7))
* **test:** stop test tilrd servers when their test dies ([9127e3d](https://github.com/tilr-dev/tilrd/commit/9127e3d1a173556243560fa343392673bd1744c1))
* **test:** wait for the scroll frame after reload and resize in the wrapped-line scroll test ([97b5f26](https://github.com/tilr-dev/tilrd/commit/97b5f261cc96c88c41e45d29fead55cdd5321c78))
* **tilr:** check who serves the local socket; stored token to loopback only ([8274039](https://github.com/tilr-dev/tilrd/commit/8274039d227a9040f6223e0cc4b312fdfe866038))
* **tilr:** end a superseded attach; README and USE_CASES match the code ([e70d499](https://github.com/tilr-dev/tilrd/commit/e70d499f3844710de4098b7568bd73fc9db871d2))
* **tilr:** survive reconnect errors, detach at once, restore on signals ([d7d4e65](https://github.com/tilr-dev/tilrd/commit/d7d4e6502f99e80aed51438920369298b7d1bd88))
* **tls:** close RUSTSEC-2026-0381/0382 with an explicit bundled-roots websocket connector ([f9ed429](https://github.com/tilr-dev/tilrd/commit/f9ed4299efd43d27cc6a879ec12f6f739e471e15))


### Performance Improvements

* **build:** link libghostty-vt through pkg-config from a shared per-machine build ([6d097ab](https://github.com/tilr-dev/tilrd/commit/6d097abe4461cb90ec375b01bb3d8925a660469e))
* **build:** optimize the compression crates in dev builds ([d8f6585](https://github.com/tilr-dev/tilrd/commit/d8f658515972140bd731152a2c74c24c985bc635))
