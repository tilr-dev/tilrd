# Proposal: Joining several servers behind one client connection

Блокер: Who joins the machines? Options: (a) the client keeps one connection per server and merges lists (no server change); (b) one tilrd relays other tilrds' panes (server-to-server links, one client connection); (c) Cloud relays. Recommendation: (a) first — no new trust paths; consider (b) only if mobile clients need one connection.

## Why

"The client holds one connection, the system joins machines": the mechanism for combining cores is undecided; `server_id` is the base (Basic Memory `main/projects/tilr/tilr-vs-herdr-0.9-razbor-connecting-the-machines-i-sravnenie-arkhitektur`, `main/projects/tilr/tilr-udalionnyi-dostup-k-core-cherez-cloudflare-tunnel-access-akkaunt-polzovatelia`). A related idea is a direct LAN connection over the tunnels (Plex-style certificates on `192-168-1-5.<id>.domain`) (`main/projects/tilr/tilr-ideia-priamoe-soedinenie-po-lokalnoi-seti-poverkh-tunnelei`).

## What Changes

- To be designed: one server relays the panes of others (pane ids are UUIDs, so they do not clash), or the client multiplexes several connections and Cloud only lists machines.

## Capabilities

### Modified Capabilities
- `protocol`
- `panes`
- `cloud-tunnels`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- The client UI for several machines.

## Impact

Depends on the design; a relay would add routing state that must survive a reload.
