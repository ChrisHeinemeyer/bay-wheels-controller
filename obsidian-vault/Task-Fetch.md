---
tags: [task, wifi, networking]
---

# `fetch_task` (`src/tasks/fetch.rs`)

Polls `https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json` (the entire Bay
Wheels network's live station status — hundreds of stations) every 120 seconds
(success path), forever.

**Rewritten (2026-08-04, `light-sleep-wifi-cycle` branch).** This task now owns the
`WifiController` outright (moved into it from `main.rs`) and fully tears the radio
down between fetches — `stop_async()`, not just `WIFI_PS_MAX_MODEM` — instead of
keeping one TLS connection alive across the idle gap. See [[Power-Management]] for
why: this Rust WiFi/RTOS stack has no `esp_pm`-style automatic-light-sleep
coordination, so a future CPU light-sleep idle hook is only safe once the radio is
completely stopped, not merely modem-sleeping. `wifi_connect_task` no longer exists
as a separate task — [[Task-Wifi-Connect]] is now just `setup()`/`connect()` helper
functions this task calls directly.

## Structure: one loop, full cycle every time

```
loop {
    controller.start_async()
    wifi_connect::connect(&mut controller, tx_power)   // scan-free reconnect using setup()'s config
    controller.set_power_saving(PowerSaveMode::None)   // DHCP + handshake need the modem awake
    stack.wait_config_up() with a 20s timeout          // DHCP renegotiates every cycle now
    do_one_fetch(...)                                  // fresh TCP+TLS handshake, one GET, stream+parse
    controller.disconnect_async() + stop_async()       // ALWAYS, regardless of outcome
    step or reset tx_power/success_streak
    Timer::after(120s success / 60s failure)           // radio is off here — Stage 2's light-sleep target
}
```

The old "outer reconnect loop / inner reuse-many-fetches loop" split is gone — the
inner loop's entire premise (skip repaying the TLS handshake by keeping a
connection alive across the gap) no longer holds once the radio is stopped every
cycle. There's nothing left to reuse, so every cycle pays the full handshake cost;
in exchange, the idle gap has no association to protect and can (eventually, once
Stage 2 lands) go into real CPU light sleep. All three retry sites — connect
failure, DHCP timeout, and connection-lost — get the same full teardown before
their backoff sleep, not just the success path.

## Per-cycle sequence (`do_one_fetch` / `fetch_once`)

1. `client.resource(HOST_URL)` — a fresh DNS+TCP+TLS handshake every cycle now (no
   more persistent-connection reuse).
2. `resource.get(PATH).headers(...).send(&mut headers_buf)` — one HTTP/1.1 request.
3. Stream the response body in 1 KiB chunks through `StreamingStationParser` (see
   [[Task-Station-Parser]]), filling a fresh `[StationData; STATION_DATA_LEN]`
   array (610 slots) indexed by `station_idx` ordinal.
4. On stream completion: `STATION_DATA_SIGNAL.signal(...)`, stamp
   `STATUS.last_fetch_at`, `FETCH_SIGNAL.signal(now)` — unchanged, so
   [[Task-Station-LEDs]]' `fetch_age` display and startup breathing animation need
   no changes.

Any transport-level error (`.send()` failing, or a body-read error mid-stream)
returns `FetchOutcome::ConnectionLost`, which still gets the same full-teardown
treatment as a normal cycle before its (shorter, 60s) backoff.

## Buffers

Same three large static buffers as before, allocated once via `StaticCell` outside
the loop (never freed — this is the only fetch task and it runs forever):
`TcpClientState<1, 32768, 32768>`, `TLS_RX_BUFFER`/`TLS_TX_BUFFER` (32 KiB each).
That's still 96 KiB of static RAM dedicated to this task's networking. Unlike
before, these buffers are no longer "reused across many cycles until the
connection drops" — a fresh `TlsConfig`/`HttpClient`/`resource` is built every
single cycle now, since there's no persistent connection left to reuse; the
buffers themselves are still allocated once and just get rewritten each cycle.

## Cost per cycle now vs. the persistent-connection design

The persistent-connection-reuse optimization (added 2026-07-04, see git history)
is intentionally given up here: every cycle now pays a full DNS+TCP+TLS handshake
(asymmetric crypto, the most CPU-expensive thing this firmware does) instead of
reusing a session across the idle gap. The trade is deliberate — see
[[Power-Management]]'s writeup on why full radio teardown should net a bigger win
than keeping the connection alive with modem-sleep, and the PPK2 verification plan
for confirming that on real hardware before trusting it. The GBFS feed itself is
still fetched in full every cycle regardless (no conditional-GET opportunity —
`TARGET_STATIONS` is ~611 entries, essentially the whole network, and the feed
regenerates every 60s anyway; see [[Power-Management#Not yet done]]).
