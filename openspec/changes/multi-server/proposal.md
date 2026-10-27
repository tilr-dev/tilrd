# Proposal: Joining several machines in the client

Client-side work: no tilrd change. The Tilr apps' part belongs in their repositories; a multi-machine `tilr` (crates/tilr, this repo) would be its own change.

## Why

"The client holds one connection, the system joins machines": the mechanism for combining cores was undecided; `server_id` is the base (Basic Memory `main/projects/tilr/tilr-vs-herdr-0.9-razbor-connecting-the-machines-i-sravnenie-arkhitektur`, `main/projects/tilr/tilr-udalionnyi-dostup-k-core-cherez-cloudflare-tunnel-access-akkaunt-polzovatelia`). A related idea is a direct LAN connection over the tunnels (Plex-style certificates on `192-168-1-5.<id>.domain`) (`main/projects/tilr/tilr-ideia-priamoe-soedinenie-po-lokalnoi-seti-poverkh-tunnelei`).

## Decision (2026-10-18)

- The client joins machines: one connection per tilrd, pane lists merged in the client, the machine list from the client's configuration or Cloud discovery. No server change, no new trust paths.
- Rejected: tilrd relaying other tilrds' panes (server-to-server links and routing state).
- Rejected: Cloud relaying (contradicts "Cloud does not see session traffic").

## What Changes

- Nothing in tilrd. What a client needs is already there: `HelloAck.server_id` (a lasting UUID per machine, `panes::a_server_keeps_its_id_when_it_restarts`) tells machines apart and finds the same machine reached by two addresses; pane ids are UUIDs, so merged lists do not clash. A display name comes from the client's configuration or Cloud, not from the server.

## Capabilities

### Modified Capabilities
- None (`skip_specs: true`).

## Non-goals

- The client UI for several machines; Cloud discovery API.
- A server-provided machine name in `HelloAck` (add later, by its own change, if a client needs it).
- Direct LAN connections over tunnels.

## Impact

None in this repository. Nothing crosses a reload.
