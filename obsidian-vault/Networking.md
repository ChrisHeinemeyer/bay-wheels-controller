---
tags: [networking]
---

# Networking (`src/network.rs`)

Thin setup layer over `embassy-net`:

- `create_network_stack` wires an `esp_radio::wifi::WifiDevice` into an `embassy_net::Stack` + `Runner` pair, using `StackResources<3>` (room for 3 sockets — [[Task-Fetch]] uses one TCP client; DNS resolution shares the stack's built-in resolver rather than consuming a separate socket slot the same way).
- `dhcp_config()` just returns `Config::dhcpv4(Default::default())` — no static IP option, no custom DNS servers, no DHCP options tuning (e.g. no way to influence lease/renewal timing from this code).
- The `Stack`/`Runner` are split at creation: `Runner` is driven by the standalone `net_task` in `main.rs` (`runner.run().await`, an infinite loop with all the actual packet pump work), while the `&'static Stack` reference is handed to [[Task-Fetch]] to do socket I/O against. This split is the standard embassy-net pattern — the runner task must always be running for the stack to make progress at all, independent of whether anyone's actively using a socket.

## What's *not* here

- No mDNS, no NTP/SNTP (timestamps used in the firmware, e.g. `Instant::now()` for fetch-age, are all relative to boot — `embassy_time::Instant`, not wall-clock time).
- No retry/backoff policy beyond what [[Task-Wifi-Connect]] does at the WiFi layer — if DHCP itself fails after a WiFi (re)connect, there's no explicit handling visible in this module; [[Task-Fetch]] just blocks on `stack.wait_config_up()` indefinitely.
