## ADDED Requirements

### Requirement: Instances survive a tilrd reload
On a reload the server SHALL hand its instances, their connections, tokens and subscriptions to the successor: the instances' pids stay the same, they stay `up`, keep their subscriptions and are routable after the reload; calls in flight at the exec SHALL be answered `unavailable` and their callees sent `Cancel`, and open streams SHALL end with `unavailable`.

**Verified by**: `reload::bus_instances_survive_a_reload`.

#### Scenario: Same pid after reload
- **WHEN** `t-1` is up and subscribed to `pane.*`, the server gets SIGUSR1, and the owner reconnects and creates a pane
- **THEN** `tilr bus ls` shows `t-1` `up` with the same pid and `restarts` unchanged, `t-1` receives `pane.created`, and `tilr bus call test.echo x` prints `x`
