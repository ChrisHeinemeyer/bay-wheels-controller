---
tags: [telemetry, reliability]
---

# Telemetry and self-recovery

Added 2026-10-12 after the board needed power-cycling every few hours (touches dead, so a hang or panic, not a stuck fetch).

## Self-recovery (`src/watchdog.rs`, always on)

- **Panic handler resets** instead of `loop {}`. It records the location and message in RTC fast RAM (`#[ram(unstable(rtc_fast, persistent))]`, which survives software resets), and the next boot logs `Previous boot reset itself: …`. esp-hal turns stack-guard hits and unhandled exceptions into panics, so those are covered too.
- **RTC watchdog (RWDT), 60s.** It runs on the RTC slow clock, so it keeps counting through light sleep, unlike the TIMG watchdogs ([[Power-Management]]). `main`'s keep-alive loop feeds it every 10s, but first resets on purpose, with a record, if the input task's heartbeat stopped or there has been no successful fetch for 15 min. A hang that stops the executor stops the feeding.
- `Reset reason: … (boot #N since power-on)` is logged at every boot. A watchdog reset leaves no record of its own, only the reset reason.
- Fetch now has a 90s overall timeout. The body read had none.

## Telemetry (`log-server` feature, `src/tasks/telemetry.rs`)

Each fetch cycle POSTs a JSON report to `LOG_SERVER` from `.env`, after the fetch and before teardown, so it adds no wakeups. It uses plain HTTP/1.1 straight over the fetch task's `TcpClient` socket: no DNS, no reqwless, 5s timeout, failures ignored. Fields: board, fw, boot count, uptime, battery % and ADC mV, RSSI, TX power, fetch ok and totals, data age, per-cycle connect/DHCP/fetch ms, previous cycle's total radio-on ms, light-sleep totals, free heap, and the reset summary (until it has been delivered once).

The receiver is a Home Assistant local add-on in `homeassistant/` (deployed to HA OS `/local_apps` via `homeassistant/deploy.sh`), status page at `http://10.0.0.107:8099/`.

## Stack budget

The main stack is whatever DRAM is left after statics. Reading `PanicInfo` at all makes every panic site's formatting live, and esp-hal's linker script puts the `Debug` lookup tables that pulls in into `.data` (~6KB). That was paid for by cutting the fetch task's TCP **TX** buffer from 32KB to 4KB (it only carries the GET, the TLS handshake and the telemetry POST). The main stack went from ~114KB to ~136KB, which ends the <7KB-headroom problem noted in [[Power-Management]].
