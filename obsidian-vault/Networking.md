---
tags: [networking]
---

# Networking (`src/network.rs`)

Thin setup layer over `embassy-net`:

- `create_network_stack` wires an `esp_radio::wifi::WifiDevice` into an `embassy_net::Stack` + `Runner` pair, using `StackResources<3>` (room for 3 sockets — [[Task-Fetch]] uses one TCP client; DNS resolution shares the stack's built-in resolver rather than consuming a separate socket slot the same way).
- `dhcp_config()` returns DHCP with the DISCOVER retry cut from smoltcp's 10s to 250ms. The radio comes up fresh every cycle and the first DISCOVER is often lost.
- [[Task-Fetch]] reuses the last DHCP lease as a static config until 90% of its renewal time T1 (the XB8 gives 48h leases), so most cycles skip DHCP entirely.

## Patched smoltcp (`patches/smoltcp-0.12.0`, v1.4.0)

Vendored through `[patch.crates-io]`; every change is marked `PATCHED (bay-wheels-controller)`:

- **DHCP DISCOVER retries keep their transaction ID.** Upstream draws a new xid per retry, so with a 250ms retry an OFFER that arrived after the next retry was dropped as stale (DHCP took 6s).
- **`LAST_LEASE_SECS` / `LAST_RENEW_SECS`** expose the lease length and T1 from the ACK, which embassy-net doesn't pass on. Lease reuse needs them.
- **ARP cache kept across same-address reconfigs.** `update_ip_addrs` flushed the neighbor cache on every call, and embassy-net re-applies the same static config before each connect, so every cycle started by ARPing again. Now it flushes only when the address list changes (ignoring transitions through "no address").
- **ARP timing:** entries live 1h (was 60s); the cache-level and per-socket retry gaps are 200ms (were 1s). A lost ARP exchange had held a socket's packets for a full second.
- The `Stack`/`Runner` are split at creation: `Runner` is driven by the standalone `net_task` in `main.rs` (`runner.run().await`, an infinite loop with all the actual packet pump work), while the `&'static Stack` reference is handed to [[Task-Fetch]] to do socket I/O against. This split is the standard embassy-net pattern — the runner task must always be running for the stack to make progress at all, independent of whether anyone's actively using a socket.

## What's *not* here

- No mDNS, no NTP/SNTP (timestamps used in the firmware, e.g. `Instant::now()` for fetch-age, are all relative to boot — `embassy_time::Instant`, not wall-clock time).
- No retry/backoff policy beyond what [[Task-Wifi-Connect]] does at the WiFi layer — if DHCP itself fails after a WiFi (re)connect, there's no explicit handling visible in this module; [[Task-Fetch]] just blocks on `stack.wait_config_up()` indefinitely.
