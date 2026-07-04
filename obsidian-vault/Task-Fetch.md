---
tags: [task, wifi, networking]
---

# `fetch_task` (`src/tasks/fetch.rs`)

Polls `https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json` (the entire Bay Wheels network's live station status — hundreds of stations) every 60 seconds, forever.

## Per-cycle sequence

1. `esp_wifi_set_ps(WIFI_PS_NONE)` — force the modem fully awake. The comment at `fetch.rs:64` explains why: `WIFI_PS_MAX_MODEM`'s ~1s sleep windows cause DNS UDP replies and TLS handshake packets to get dropped.
2. Build a fresh `TlsConfig` (seed `0`, `TlsVerify::None` — **no certificate verification**, see [[Open-Questions]]) and a fresh `HttpClient` over the shared `TcpClient`/`DnsSocket`.
3. GET the URL, full TLS handshake from scratch (no session resumption, no connection reuse across cycles).
4. Stream the response body in 1 KiB chunks through `StreamingStationParser` (see [[Task-Station-Parser]]), filling a `[StationData; STATION_DATA_LEN]` array (610 slots) indexed directly by `station_idx` ordinal.
5. On stream completion: `STATION_DATA_SIGNAL.signal(...)`, stamp `STATUS.last_fetch_at`, `FETCH_SIGNAL.signal(now)`.
6. `esp_wifi_set_ps(WIFI_PS_MAX_MODEM)` — let the modem sleep again.
7. `Timer::after(60s)`.

## Buffers

Three large static buffers, allocated once via `StaticCell` (never freed, by design — this is the only fetch task and it runs forever):
- `TcpClientState<1, 32768, 32768>` — 1 concurrent connection, 32 KiB TX/RX.
- `TLS_RX_BUFFER` / `TLS_TX_BUFFER` — 32 KiB each, sized for the TLS handshake plus streaming.

That's 96 KiB of static RAM dedicated to this one task's networking, out of a 64 KiB heap allocator elsewhere (`main.rs:76`) plus whatever's statically allocated — worth remembering if you ever need to grow another task's buffers and start hitting link-time RAM limits.

## Why this task owns WiFi power-save mode

`wifi_connect_task` explicitly does *not* touch `esp_wifi_set_ps` (see [[Task-Wifi-Connect]]) — this task is the sole owner of that toggle, flipping to `NONE` only for the duration of the actual HTTP round-trip and back to `MAX_MODEM` for the 60s idle gap. This is the main mechanism by which the device saves radio power day-to-day; see [[Power-Management]].

## Cost per cycle (why this matters for power)

Every 60s, this task pays for:
- A full DNS lookup + TCP handshake + TLS handshake (asymmetric crypto — the most CPU-expensive thing this firmware does) — all while the modem is forced fully awake.
- Downloading the **entire** network-wide GBFS feed, not just the ~120 stations in `TARGET_STATIONS` (see [[Data-Model]]) — GBFS doesn't support server-side filtering, so there's no way to ask for less without a proxy. The streaming parser at least avoids buffering the whole response in RAM, but it doesn't reduce bytes-over-the-air.

Neither of these behaviors have obvious cheap fixes given GBFS's API shape, short of adding a middleman server or accepting slightly stale data via conditional GETs (ETag/If-Modified-Since) to skip re-downloading unchanged bodies.
