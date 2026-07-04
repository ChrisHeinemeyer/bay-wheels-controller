---
tags: [task, wifi, networking]
---

# `fetch_task` (`src/tasks/fetch.rs`)

Polls `https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json` (the entire Bay Wheels network's live station status — hundreds of stations) every 60 seconds, forever.

**Updated (2026-07-04)**: this task now (1) reuses one TLS connection across many fetch cycles instead of reconnecting every time, and (2) lets the modem sleep (`WIFI_PS_MAX_MODEM`) for the entire life of that connection, not just the 60s idle gap — `WIFI_PS_NONE` is now confined to just the initial connect/handshake. Both changes verified on real hardware. See [[Power-Management#Already implemented]].

## Structure: outer connect loop, inner reuse loop

```
outer loop {                              // (re)establish a connection
    PS_NONE (DNS + TLS handshake needs the modem fully awake)
    client = HttpClient::new_with_tls(...)
    resource = client.resource(HOST_URL).await   // one TCP+TLS handshake
    PS_MAX_MODEM                          // modem sleeps for the rest of this connection's life

    inner loop {                          // reuse `resource` for repeated GETs
        fetch_once(&mut resource)         // GET PATH, stream+parse body — modem stays asleep
        if ConnectionLost: sleep 60s, break to outer loop (reconnect)
        else: sleep 60s, loop inner again (same connection)
    }
}
```

`HOST_URL = "https://gbfs.lyft.com"` and `PATH = "/gbfs/2.3/bay/en/station_status.json"` are split apart specifically so `client.resource(HOST_URL)` connects once against the bare origin (`base_path` ends up `"/"`) and each subsequent `resource.get(PATH)` sends the full path per-request without reconnecting — see reqwless's `HttpResource` (`resource()`/`.get()`/`.post()` etc.), which is designed exactly for this "connect once, request many times" pattern (HTTP/1.1 persistent connections, no explicit keep-alive header needed).

## Why it can still fall back to reconnecting

GBFS's server has no obligation to hold an idle connection open for a full 60s gap between fetches — it may close it, time it out, or reset it at any point. `fetch_once` returns `FetchOutcome::ConnectionLost` on **any** transport-level error (the `.send()` call failing, or a body-read error mid-stream), and the inner loop treats that as "this connection is dead" — it sleeps 60s then breaks back out to the outer loop, which throws away the old `client`/`resource`/`tls_config` and does a full fresh connect. This makes the persistent-connection optimization purely additive: best case, the connection survives many 60s gaps and most cycles skip the handshake entirely; worst case (server closes it every time), behavior degrades to exactly the old always-reconnect behavior, just with one extra 60s sleep before the retry.

## Per-successful-cycle sequence (`fetch_once`)

The modem is **not** touched at all inside `fetch_once` anymore — it stays in whatever power-save state the outer loop set (`MAX_MODEM`, once past the initial connect). Verified on real hardware that a plain GET+response over a reused TLS connection tolerates this fine (see [[Power-Management#Already implemented]]).

1. `resource.get(PATH).headers(...).send(&mut headers_buf)` — no new DNS lookup, no new TLS handshake, just an HTTP/1.1 request over the already-open encrypted socket, sent while the modem is asleep in `MAX_MODEM`.
2. Stream the response body in 1 KiB chunks through `StreamingStationParser` (see [[Task-Station-Parser]]), filling a fresh `[StationData; STATION_DATA_LEN]` array (610 slots) indexed directly by `station_idx` ordinal — this array is still recreated every cycle (not carried across), so a station that disappears from one response resets to `Default` for that cycle same as before.
3. On stream completion: `STATION_DATA_SIGNAL.signal(...)`, stamp `STATUS.last_fetch_at`, `FETCH_SIGNAL.signal(now)`.

## Buffers

Same three large static buffers as before, allocated once via `StaticCell` (never freed, by design — this is the only fetch task and it runs forever):
- `TcpClientState<1, 32768, 32768>` — 1 concurrent connection, 32 KiB TX/RX.
- `TLS_RX_BUFFER` / `TLS_TX_BUFFER` — 32 KiB each, sized for the TLS handshake plus streaming.

That's 96 KiB of static RAM dedicated to this one task's networking, out of a 64 KiB heap allocator elsewhere (`main.rs:76`) plus whatever's statically allocated — worth remembering if you ever need to grow another task's buffers and start hitting link-time RAM limits. `tls_read_buffer`/`tls_write_buffer` are borrowed fresh by a new `TlsConfig` each time the outer loop reconnects — the borrow-checker enforces that the *previous* `client`/`resource` (which held the prior borrow) has already gone out of scope by then, which is exactly what happens when the inner loop `break`s.

## Why this task owns WiFi power-save mode

`wifi_connect_task` explicitly does *not* touch `esp_wifi_set_ps` (see [[Task-Wifi-Connect]]) — this task is the sole owner of that toggle. This is the main mechanism by which the device saves radio power day-to-day; see [[Power-Management]].

## Cost per cycle now vs. before

Before this change, every 60s cycle paid for a full DNS lookup + TCP handshake + TLS handshake (asymmetric crypto — the most CPU-expensive thing this firmware does), all while the modem was forced fully awake for the whole exchange. Now, as long as the connection survives the idle gap, a cycle is just one HTTP GET + response over an already-open socket — no DNS, no handshake, no new asymmetric crypto. The one cost that's unchanged regardless of connection reuse: downloading the **entire** network-wide GBFS feed every cycle. Note this isn't actually "downloading everything to use a small fraction" — `TARGET_STATIONS` turns out to hold 611 entries (essentially every station in the network, see [[Data-Model#TARGET_STATIONS]]), so there's no server-side filtering to wish for even in principle; this device genuinely wants nearly all of the feed. Conditional GET (`If-None-Match`/`If-Modified-Since`) was tested directly against the live endpoint and ruled out for a different reason: the feed's `ttl` is 60s and it regenerates every 60s with a fresh `last_reported` timestamp per station, so at this poll cadence a conditional GET would almost never actually return a `304`. See [[Power-Management#Not yet done]] for the full writeup.
