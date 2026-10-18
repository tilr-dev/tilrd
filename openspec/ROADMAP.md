# tilrd roadmap

The order in which the open changes in `openspec/changes/` are to be implemented. Written
2026-10-18 against dev at 1163abd; every change in phases 1-5 was checked against the code
there. `openspec apply` and `openspec archive` keep this file current (rule in
`config.yaml`): a change that is archived is removed from its phase and listed under "Done"
with its commit; a change whose status, dependencies or blocker changes is updated here.

Columns: **size** S (a day or less), M (a few days), L (a week or more); **depends on** the
change's `Зависит от:` line; **blocker** whether its proposal starts with a `Блокер:` line
(a question only the owner can answer); **status** `ready` (can be applied now, once its
dependencies are archived), `needs owner` (answer the Блокер first), `deferred`, `design
first`.

## Release gate

Before dev is merged into `main` and the first archive release is cut (each needs the
owner's word):

- Phase 1 in full (correctness and security of what the first release ships).
- `protocol-v3` (the wire contract the first release's clients speak) and `non-latin-ctrl-alt`.
- `readme-code-reconciliation` (README is the user-facing contract).
- `release-process-tests`, `scoped-cdn-token` (before the `cdn` job's first run),
  `cloud-default-url` (machines linked by the first release store the Cloud URL).
- `installer-atomic-install`: the first archive release is blocked by it. tilr.sh serves a
  placeholder and the installer in the sh directory is still the old tilr-server one;
  decided 2026-10-18: `install.sh` lives here (`scripts/install.sh`) and the `cdn` job uploads it.
- `tilr-mac-rename-compatibility`: the desktop-mac rename branch and its app feed land in the
  same step (the app's own repository; needs `protocol-v3`).

Phases 3 (test-coverage-gaps) and 5 do not gate the release.

## Phase 1: correctness and security

All of phase 1 is done (see "Done").

## Phase 2: protocol v3 and input

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| reload-screen-fidelity | carry soft wrap, pending wrap, the ClosePane deadline and kitty placements across a pane reload | M | - | no | ready |

## Phase 3: docs and tests

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| readme-code-reconciliation | README / USE_CASES match the code; a superseded `tilr attach` ends instead of reconnecting | S | - | no | ready |
| test-coverage-gaps | tests for the 19 untested server/client requirements; fix or quarantine the known flakes | L | - | no | ready |

## Phase 4: release infrastructure

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| release-process-tests | tests for `scripts/release.sh` (stand-in wrangler, local CDN) and static checks of the workflows | M | - | no | ready |
| scoped-cdn-token | the `cdn` environment gets a token scoped to the tilr-cdn bucket | S | - | no | ready (owner's Cloudflare access to create the token) |
| cloud-default-url | `https://wss.tilr.dev` becomes the default Cloud URL | S | - | no | ready |
| installer-atomic-install | the archive installer served at tilr.sh with the first release | S | release-process-tests | no | ready |
| tilr-mac-rename-compatibility | the Tilr app moves to tilrd (tracking; work in desktop-mac) | S here, M there | - | no | ready |

## Phase 5: features

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| log-level-from-env | `RUST_LOG` sets tilrd's log filter | S | - | no | ready |
| pane-activity-notifications | `PaneActivity` push: bell and foreground command finished | M | - | no | ready |
| pane-process-title | `ps` shows each pane's shell, cwd and command, kept across a reload | M | - (shares the `tcgetpgrp` check with pane-activity-notifications) | no | ready |
| osc8-hyperlinks | OSC 8 links in frames (`Frame.links`, `Run.link`); `tilr attach` draws them | S/M | - | no | ready |
| reverse-clipboard | a copy in a pane (OSC 52, xclip, wl-copy) reaches the watching clients | M | - | no | ready |
| files-and-edit-followups | `tilr edit --wait` (usable as GIT_EDITOR); `tilr edit` outside the shared directories refused at once | M | - | no | ready |
| tilr-deploy | `tilr deploy user@host` installs and starts tilrd over the user's ssh | M | installer-atomic-install | no | ready (its `--cloud` part is not designed; after Cloud is in production) |
| reload-exec-checked-file | the executable a reload checks is the one it execs (Linux `/proc/self/fd`, macOS a private copy) | M | - | no | ready |
| metrics-no-hello-and-prefix | no-Hello closes counted in the drops metric (`no_hello`); one metric prefix `tilrd_` (the twelve `tilr_*` families renamed), **BREAKING** for dashboards | S | - | yes (prefix) | ready |

## Phase 6: big directions (design first)

The owner designs these before any implementation. Whether QUIC comes from iroh or quinn is
being researched (Basic Memory `main/projects/tilr/tilr-iroh-kak-transport-dlia-vsego-vopros-na-prorabotku`);
the answer shapes `quic-transport`, `directory-sync` and `port-forwarding`.

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| quic-transport | a QUIC/UDP transport for bad networks, one stream per lane | L | - | - | design first |
| directory-sync | `tilr-sync` crate embedded in tilrd: push sync over QUIC | L | - | - | design first |
| multi-server | the client joins several machines (no tilrd change) | L | - | - | design first |
| port-forwarding | `ssh -R` / `-L` style forwards; waits for the transport decision (iroh vs quinn); when built, off by default like SOCKS | L | the transport decision | no | design first |
| itests-stage-5 | itests group H (load, security), distributions, macOS backend | L | test-coverage-gaps | - | design first |

## Deferred

| change | goal | size | depends on | blocker | status |
|---|---|---|---|---|---|
| end-to-end-encryption | security design for E2E between client and server | L | - | - | deferred |
| sandboxed-pane | panes in a sandbox for agents (App Sandbox, Landlock) | L | - | - | deferred |
| public-repo-readiness | checklist for making the repository public | S | readme-code-reconciliation | - | deferred |
| release-platforms | musl, macOS on the CDN, x86_64 builds | M | public-repo-readiness | - | deferred |
| read-only-viewer | a viewer token gives read-only connections; deferred until a use is named (design ready: per-server viewer token) | M | - | no | deferred |
| libghostty-upstream-fixes | report and carry fixes for libghostty-vt (also: OSC 8 in the reload snapshot, pending-wrap accessor) | S | - | - | deferred |

## Done

| change | commit |
|---|---|
| openspec-setup | 27be1ea, 40d6057, 552dcd9 |
| reload-rollback | 4298b15 |
| reload-signals-and-socket | a0f1ae9 |
| bulk-window-stall | 7d18ae3 |
| exec-escaped-descendants | 48efd35 |
| client-socket-peer-check | 8274039 |
| hello-deadline | f32643d |
| host-socket-per-process | ddcc453 |
| notify-socket-env | 4e7812b |
| attach-robustness | d7d4e65 |
| sighup-ignoring-shell-reaped | 47bce42 |
| cloudflared-outlives-exec | 0aff8e0 |
| durable-file-writes | 42a4cc2 |
| service-install-ownership | 1d44e4b |
| shims-path-robustness | 0590786 |
| startup-signal-window | f8fb63b |
| pane-socket-peer-check | 248dd56 |
| protocol-v3 | 3dc42e2 |
| non-latin-ctrl-alt | 92e4f43 |

Older work is in `openspec/changes/archive/`.
