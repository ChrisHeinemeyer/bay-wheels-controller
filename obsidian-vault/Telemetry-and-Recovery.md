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

Each fetch cycle POSTs a JSON report to `LOG_SERVER` from `.env`, after the fetch and before teardown, so it adds no wakeups. It uses plain HTTP/1.1 straight over the fetch task's `TcpClient` socket: no DNS, no reqwless, 5s timeout, failures ignored. Fields: board, fw, boot count, uptime, battery % and ADC mV, RSSI, TX power, fetch ok and totals, data age, per-cycle connect/DHCP/fetch ms, previous cycle's total radio-on ms, light-sleep totals, free heap, and the reset summary (until it has been delivered once). Since v1.4.0 also the fetch sub-phases (`handshake_ms`, `ttfb_ms`, `body_ms`, `parse_ms`, `body_bytes`), `src` (mirror/lyft), `lease` (reused/dhcp) and `lease_s`; with `sleep-trace`, per-task embassy polls while asleep.

The receiver is a Home Assistant local add-on in `homeassistant/` (deployed to HA OS `/local_apps` via `homeassistant/deploy.sh`), status page at `http://10.0.0.107:8099/`. Since add-on 1.1.0 it also serves the GBFS mirror (`GET /gbfs/station_status.json`, see [[Task-Fetch]]). The repo has 1.1.2; HA runs 1.1.1, which still has a diagnostic UDP echo on 8099/udp that nothing uses any more.

## Stack budget

The main stack is whatever DRAM is left after statics. Reading `PanicInfo` at all makes every panic site's formatting live, and esp-hal's linker script puts the `Debug` lookup tables that pulls in into `.data` (~6KB). That was paid for by cutting the fetch task's TCP **TX** buffer from 32KB to 4KB (it only carries the GET, the TLS handshake and the telemetry POST). The main stack went from ~114KB to ~136KB, which ends the <7KB-headroom problem noted in [[Power-Management]].

## Backtraces (v1.3.1)

The panic handler also walks the Xtensa register-window stack and saves up to 16 return addresses with the crash record. The walker is vendored from esp-backtrace 0.18.1, because the crate insists on a println/defmt backend. The next report sends them as `backtrace`. `scripts/flash.sh` (the cargo runner) archives every flashed ELF as `firmware-archive/<git describe>.elf`. `uv run scripts/symbolize.py` matches a report's `fw` to that ELF and prints function + file:line. File:line comes from scanning the DWARF line tables with pyelftools, because in these fat-LTO builds the address index is wrong and addr2line / llvm-dwarfdump return `??`. CPU exceptions and stack-guard hits still only point into esp-hal's handler.

## The 2.5-hour out-of-memory reset (fixed in v1.3.1)

Telemetry showed every reset was `memory allocation of 40 bytes failed` after ~8,900s, with free heap at cycle start falling ~200 bytes per cycle. Cause: esp-rtos 0.2.0 `Timer::delete` (esp-radio's timers for the WiFi blob) marked every timer that wasn't in the queue as "drop later" and forgot it. Only timers the queue was processing ever got dropped, so a deleted one-shot that had already fired leaked ~48 bytes. The blob creates and deletes several per connect cycle, and that only began when Stage 1 started tearing WiFi down every cycle. Upstream fixed it in esp-rtos 0.3.0 (#4541), but that needs esp-hal 1.1. So `patches/esp-rtos-0.2.0` carries a one-line backport via `[patch.crates-io]`; drop it once on esp-rtos >= 0.3.0. Confirmed on hardware: free heap at cycle start is flat.
