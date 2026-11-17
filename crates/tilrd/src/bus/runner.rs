//! Spawning and supervising the instances of `bus.toml` (B10): own session, fd 3 or
//! an instance socket, restarts with backoff, attached to the broker with
//! `BrokerHandle::attach_stream`.
//!
//! Owner: `bus-instances`. A stub of `bus-broker-core` until then.
