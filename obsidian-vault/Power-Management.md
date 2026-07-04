---
tags: [power]
---

# Power Management

This is a 3× AA battery product ([[Task-Battery]]), so every task's polling cadence and every radio state transition is effectively a battery-life decision. Recent git history (`b128491`, `3733354`, `95bdb66`, `572d248`, `3a52045`) shows active, ongoing tuning in this area.

## Already implemented

- **CPU clock reduced to 80 MHz** (`main.rs:72`) instead of the ~240 MHz default.
- **No BLE** — `esp-rtos`/`esp-radio` features only enable `wifi`, not `ble` ([[Build-and-CI#Cargo features]]/`Cargo.toml`).
- **Adaptive WiFi TX power** — starts at 21 dBm, steps down 0.5 dB every 3 minutes of stable connection to a 15 dBm floor, locks at max after any disconnect-while-stepping. See [[Task-Wifi-Connect]].
- **WiFi modem power-save owned by the fetch cycle** — `WIFI_PS_NONE` only during the actual HTTP round-trip, `WIFI_PS_MAX_MODEM` (listen_interval=10, ~1s sleep windows) for the 60s idle gap between fetches. See [[Task-Fetch]]. This split exists because enabling power-save during DNS/TLS was found to drop packets.
- **Adaptive input poll rate** — 500ms idle, 150ms for 10s after a touch. See [[Task-Input-Read]].
- **Connection-aware serial backoff** — 300ms when a host is reading the USB frame, 2s when not. See [[Task-Serial-Status]].
- **No WiFi sniffer / promiscuous mode** — removed specifically because it prevented modem sleep (see `wifi.rs`'s one-line comment and git history `3fe2665`).
- **Persistent HTTPS connection across fetch cycles.** [[Task-Fetch]] now connects once (`client.resource(HOST_URL)`) and reuses that TLS connection for repeated 60s-interval GETs instead of doing a full DNS+TCP+TLS handshake every cycle, falling back to a fresh reconnect only when the connection is actually lost (server closed it, transport error). See [[Task-Fetch#Structure: outer connect loop, inner reuse loop]] for the design and why it degrades gracefully if the server doesn't cooperate.

## Not yet done — biggest remaining opportunities

Roughly in order of likely impact, given what's implemented above:

1. ~~**Full-network GBFS download every 60s — conditional GET.**~~ **Tested directly against the live endpoint (curl, 2026-07-04) and ruled out at the current poll rate.** The feed supports `If-None-Match`/`If-Modified-Since` correctly (confirmed: matching ETag → `304`, 0-byte body vs. 271 KB for `200`; non-matching ETag → `200` as expected). But the feed declares `"ttl": 60` and regenerates on a fixed 60-second schedule — confirmed `Last-Modified` advanced exactly 60s with a new `ETag` between two consecutive checks. Every station object carries a `last_reported` Unix timestamp that's rewritten on every regeneration, so the file's content (and its ETag) changes on **every single cycle**, whether or not any station's bike count actually moved. Since this firmware polls every 60s — matching the feed's own regeneration interval — a conditional GET would almost never actually return a `304` in practice. Not worth the added complexity unless the poll interval were stretched well beyond 60s (trading data freshness for bandwidth), and even then it'd only help on cycles that happen to land between regenerations.
2. **Modem forced awake (`WIFI_PS_NONE`) for every GET, not just the handshake.** Now that [[Task-Fetch]] reuses a connection across cycles, a routine GET on an already-open TLS session involves no new DNS or handshake packets — the specific fragility that justified forcing `PS_NONE` (dropped DNS/handshake packets under `MAX_MODEM`'s sleep windows) may not apply to a plain reused-connection request/response. Worth testing empirically whether `PS_MAX_MODEM` can stay on during those GETs too; left conservative (still forcing `PS_NONE`) for now since this hasn't been validated on hardware.
3. **No CPU light-sleep.** Confirmed by grep: nothing in this codebase calls into `esp-hal`'s RTC sleep primitives (`esp_hal::rtc_cntl::sleep`, which does exist and support the S3). All the sleep here is WiFi-modem-level, not CPU-level — the CPU only idles via the Embassy executor's normal WFI between task wakeups, never enters a deeper sleep state that clock-gates more of the chip. This is a bigger lift (needs coordination with WiFi/USB/SPI peripherals staying responsive) but is the only lever that touches the CPU's own baseline draw.
4. **AL5887 chip-enable never toggled off.** [[Task-Station-LEDs]]/[[Hardware-Drivers#AL5887 LED driver]] — `set_chip_enable(true)` is called once at startup and never revisited; blanking the display (`StationIdx::None`) zeroes LED colors but leaves the driver chip fully enabled. Powering it down during extended no-touch idle would cut its quiescent draw, not just LED current.
5. **Fixed LED brightness (30/255) regardless of how many LEDs are lit.** No global dimming based on total lit-LED count, even though `LedGlobalDimming` register exists unused ([[Hardware-Drivers#AL5887 LED driver]]).

## Smaller/secondary ideas

- `listen_interval=10` (~1s modem sleep) in [[Task-Wifi-Connect]] could potentially go deeper now that the fetch cycle handles `PS_NONE` around the fragile DNS/TLS window — worth re-testing empirically.
- `battery_task` samples every 5s ([[Task-Battery]]); battery % doesn't need that resolution — 30-60s would remove a periodic wakeup for negligible loss of freshness.
- `input_read_task` polls on a fixed timer even at idle; if the shift-register hardware exposes any kind of change/interrupt line, interrupt-driven reads would remove the periodic wakeup entirely rather than just slowing it to 500ms.
- `station_leds_task`'s unconditional `Timer::after(50ms)` every loop iteration ([[Task-Station-LEDs]]) is an extra ~20Hz wakeup layered on top of the input task's own cadence.
- `dprintln!` resolves to `rtt_target::rprintln!` in normal (non-`debug-serial`) builds too ([[Build-and-CI#Cargo features]], `lib.rs`), including in per-cycle hot paths (wifi maintenance loop, battery task, fetch task) — cheap per call, but it's live in shipped builds, not compiled out.
- `serial_status_task` runs unconditionally even if no USB host will ever be attached in a standalone battery deployment; without a VBUS-presence check there's no way to skip spawning it entirely.
- Release profile uses `opt-level = 's'` (size) rather than `3` (speed) — see [[Build-and-CI#Profiles]]. Worth an A/B current-draw measurement, since fewer CPU cycles per task wakeup (shorter active windows) is a distinct axis from binary size.

## What a light-sleep design would have to account for

If CPU light-sleep is ever pursued (item 3 above), the things currently keeping the CPU "busy enough to matter" are: the six always-running tasks' timers (ranging 50ms–60s), two blocking SPI buses (100kHz, so each transfer takes real wall-clock time with the CPU parked in a blocking call, not free to sleep), and the WiFi/USB peripherals needing their own clocks active regardless of CPU sleep state. Aligning the task wakeup periods to common multiples (see [[Architecture#Task graph]]) would be a low-risk first step that makes any future light-sleep work more effective, independent of whether light-sleep itself gets implemented.
