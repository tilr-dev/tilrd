# Design

## Cause

The proxy chooses random chunk stalls with exponential backoff: consecutive losses
hold delivery for 200, 400, 800, 1600, 3200 ms and so on (capped per stall at
12800 ms). Queued input and frames wait behind these stalls in both directions.
This transport distribution supplies no five-second p90 or ten-second screen-stall
bound. The server cannot shorten a round trip imposed by the network.

The other false oracle compares server timeouts with client disconnect events.
The driver treats any incoming frame as activity, while the server requires
incoming client traffic or WebSocket pongs. An uplink stall can therefore expire
the server timeout while downlink frames still keep the client alive. The close
itself can be delayed. A server silence timeout is correct here, even before the
client records a disconnect; lag alone must still not disconnect a client.

## Controlled reproductions

- Run the original proxy recovery case through a proxy that replaces its random
  degradation with fixed 3000 ms each-way delay and no loss/jitter. It has a
  six-second input echo and no disconnect. The former p90 assertion rejects it;
  the corrected recovery case accepts it and checks input after healing.
- Freeze only the uplink after starting counter output, and delay downlink by
  3000 ms. Wait for the observer to see the viewer removed. The server logs a
  silence timeout while the client has received frames and has no disconnect
  event yet. The former drop-count ordering rejects this valid state. Assert the
  silence reason, then resume and verify the screen and one viewer recover.
- Keep the existing complete ten-input baseline/delay/recovery threshold check:
  p90 is below five seconds with zero delay, above it with 3000 ms each way,
  and below it after clearing delay. No client disconnect or server drop occurs.

## Correction

Random-loss runs retain their transport metrics and recovery assertions. Remove
absolute latency, stall and disconnect-count assertions during that random
condition: they constrain the environment, not the server. The controlled cases
check latency and liveness without relying on random outcomes, repeated runs,
sleeps for synchronization, or increased timeouts. The existing one-second
sampling cadence in the diagnostic loss windows is unchanged.

There is no server, client protocol, or requirement change and no quarantine.
The existing client-liveness distinction between lag and silence is preserved.
Historical random runs cannot identify a unique lost packet or exclude host load;
the reproductions establish that the old oracles fail on correct behavior without
host load, and the corrected oracles no longer depend on that network variation.
