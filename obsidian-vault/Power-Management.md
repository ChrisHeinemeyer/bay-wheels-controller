---
tags: [power]
---

# Power Management

This is a 3× AA battery product ([[Task-Battery]]), so every task's polling cadence and every radio state transition is effectively a battery-life decision. Recent git history (`b128491`, `3733354`, `95bdb66`, `572d248`, `3a52045`) shows active, ongoing tuning in this area.

## In progress: WiFi full teardown + CPU light-sleep (`light-sleep-wifi-cycle` branch)

Started 2026-08-04 after a Nordic PPK2 measurement showed a ~55mA sustained idle
floor (not spikes) — everything below in "Already implemented" only shaves the
active/spike current, nothing touches that baseline. Two findings changed the
design from what's described further down:

- This Rust stack (`esp-radio`/`esp-rtos`) has no `esp_pm`-equivalent PM-lock
  coordination between WiFi and CPU sleep, unlike ESP-IDF's "automatic light
  sleep" — so CPU light-sleep is only safe once the radio is **fully torn down**,
  not just modem-sleeping (`WIFI_PS_MAX_MODEM`).
- ESP32-S3 has no USB-Serial-JTAG wake source at the hardware register level
  (confirmed via esp-hal's `WakeTriggers` bitfield) — the prior light-sleep
  attempt's bricking was a structural hardware limitation, not a missed API call.

**Stage 1 (implemented and hardware-validated, commit `ad2250c`):** [[Task-Fetch]]
now owns the `WifiController` directly and fully disconnects+stops the radio
between fetch cycles instead of keeping one TLS connection alive with
modem-sleep — see [[Task-Fetch]] and [[Task-Wifi-Connect]] for the new structure.
This gives up the persistent-connection-reuse optimization below in exchange for
a much lower idle floor. Verified on real hardware across multiple consecutive
connect→DHCP→fetch→teardown→reconnect cycles (271KB body, 606 stations parsed
each time) — a full PPK2 current comparison against the 55mA baseline is still
pending.

**Stage 2 (first attempt failed on hardware; fixed as Stage 2b below):** a custom `esp-rtos` idle hook
(`src/power.rs`, gated behind a new `light-sleep` feature, off by default)
light-sleeps the CPU in fixed 20ms chunks whenever the scheduler has no ready
task — esp-rtos gives a custom idle hook no way to learn the next task deadline,
so this can't be made deadline-aware, only fixed-chunk. Given the USB-JTAG
limitation above, it registers the provisioning button (GPIO2, RTC-capable on the
S3, re-`steal()`'d after the boot-time provisioning check drops it — see
`main.rs`) as an `Ext0WakeupSource` alongside the timer — holding it forces a wake
window for probe-rs/reflashing — with GPIO0-strap-to-GND ROM recovery as the
backstop of last resort. Wired into `main.rs` via `esp_rtos::start_with_idle_hook`
in place of `esp_rtos::start`, feature-gated; `#[esp_rtos::main]` didn't need to
change.

**Stage 2 hardware test (2026-10-11): failed — the firmware freezes entirely.**
Flashed `--features use-env,light-sleep`. Not just USB-unresponsive as expected:
no RTT output at all (not even the first boot line), LED11 never breathed (it
should breathe blue until the first fetch), and holding GPIO2 did *not* bring
JTAG back. Recovery: GPIO0-to-GND + reset into ROM download mode, reflash
`--features use-env` (confirm probe-rs prints *Programming*, not just
*Verifying* — `--preverify` skips the write if flash already matches), reset
with no buttons held. The non-light-sleep build runs normally after that.
Practical gotchas hit along the way: `cargo run`'s `probe-rs run` runner never
exits on its own (Ctrl+C after `Finished in Ns`); pressing reset re-enumerates
the native USB-JTAG and kills the attached probe-rs session (re-`probe-rs
attach` afterwards); holding GPIO2 across a reset boots into provisioning mode.

**Root cause, from reading esp-hal 1.0.0 / esp-rtos 0.2.0 source:** firmware
time stops during light sleep and nothing compensates for it.

- esp-rtos's clock is `esp_hal::time::Instant::now()`, which on the S3 reads
  SYSTIMER unit 0 (`esp-hal/src/time.rs`). Scheduler wakeups are armed on the
  TIMG1 timer passed to `start_with_idle_hook`, as a one-shot *relative*
  countdown (`esp-rtos/src/timer/mod.rs`, `arm_next_wakeup`).
- Both are clocked from XTAL/APB, which the default `RtcSleepConfig` gates
  during light sleep (`xtal_fpu = false`).
- `Rtc::sleep` / `TimerWakeupSource` do no post-wake time adjustment — the
  source literally has `// TODO: maybe add sleep time adjustlemnt like idf`
  (`esp-hal/src/rtc_cntl/sleep/esp32s3.rs`). esp-rtos has no sleep awareness
  at all. ESP-IDF advances SYSTIMER by the RTC-measured sleep time on wake;
  this stack doesn't.
- Net effect: firmware time only advances in the microsecond awake slivers
  between 20ms chunks, so every `Timer::after` (30ms breathe tick, WiFi
  connect timeouts, 60s fetch) effectively never fires.

Two further design bugs in the Stage 2 hook, independent of the clock:

- It sleeps whenever the scheduler is idle, *including mid-fetch-cycle while
  the radio is up* (waiting on association/DHCP/TLS) — exactly the
  WiFi-vs-CPU-sleep case the top of this section says is unsafe on this stack.
- The GPIO2 escape hatch only *wakes* the chip; the hook loops straight back
  into sleep, so the USB PHY keeps dropping and probe-rs never gets a window.

**Stage 2b: working on hardware (2026-10-11).** Build with
`--features use-env,light-sleep`. The board sleeps ~89% of each idle stretch,
touches register while asleep, and fetches still fire every 120s of firmware
time (~1% of real time after calibration — data stays "roughly every 2
minutes"; LED11 green/yellow on touch confirms it without logs). Design, as
landed in `src/power.rs`:

1. **Radio gating** — [[Task-Fetch]] sets `power::RADIO_ACTIVE` for the whole
   start→fetch→stop window (cleared only once `is_started()` confirms the radio
   is off); the idle hook does plain `waiti` while it's set.
2. **Deadline-aware naps** — esp-rtos arms TIMG1 as a one-shot counting up
   from 0 to the next deadline, so `alarm - counter` is the time until the
   scheduler needs the CPU. Each nap ends 2ms before that (max 1s); under 5ms
   it `waiti`s instead and lets the alarm fire normally. This replaced the
   fixed 20ms chunks, which lost scheduler alarms (counter jumped onto/past
   the alarm right before re-sleeping) and froze every task during sleep.
3. **Clock compensation** — both SYSTIMER and TIMG1 lose the full nap (both
   measured). On a timer wake the nap lasted exactly the requested time, so
   both are advanced by that via raw register writes (not esp-hal's
   `set_count`, which shifts the high word the wrong way). On a GPIO2 wake the
   RTC-measured time is a fallback.
4. **RC_SLOW calibration** — esp-hal assumes a fixed 136kHz and never
   calibrates; measured ~8% fast on this board (scale ~913k-926k ppm between
   stretches). The hook compares RTC vs SYSTIMER every 0.5s during awake
   stretches (boot grace, fetch cycles, button windows) and scales nap
   requests by it.
5. **Fresh RTC latch** — on the S3 `TIME_UPDATE` has no valid bit and esp-hal
   reads straight after triggering, getting the *previous* latch. This made
   the wake alarm fire early (4ms naps for 21ms requested) until
   `latch_rtc_time()` waits for the latch first. Oddly, an RTC read right
   after waking still returns ~the pre-sleep value — unexplained, which is why
   (3) doesn't rely on it.
6. **Debug access** — no sleep for 20s after reset (`BOOT_GRACE`); a GPIO2
   press opens a 30s no-sleep window. The first nap reliably gets a spurious
   ext0 wake (enabling the RTC pull-up didn't stop it), so ext0 wakes are only
   honoured if GPIO2 actually reads low.

Wake sources while asleep, from the `sleep-trace` feature (embassy-executor
trace hooks counting per-task polls; diagnostic only):

| Source | Wakes/s | Notes |
|---|---|---|
| esp-radio / WiFi blob with radio stopped | ~16 | Timers the blob keeps firing after `stop_async()`; esp-rtos's radio timer thread only wakes for due timers, so they're real. Not in our code. |
| `station_leds` + `input_read` | ~6 | As designed: 500ms poll + 50ms LED follow-up per signal |
| `serial_status` | ~1 | Was 300ms: the flush-based "host connected" check passed with USB down. Now also requires the USB SOF frame counter to be advancing. |

**DHCP 10s → 1s (2026-10-12).** Every cycle's DHCP took ~9.9s: embassy-net
resets DHCP on each link-up (the radio is torn down every cycle), the first
DISCOVER is reliably lost, and smoltcp's default `discover_timeout` is 10s.
`network.rs` now sets it to 1s; verified on hardware (`dhcp_ms` ≈ 1.0s). Still
exactly one retry, so the first DISCOVER is still lost (maybe modem PS still on
at link-up). Reusing the lease would need the DHCP ACK, which embassy-net
discards (lease time is private in smoltcp).

Next options: fully deinit esp-radio between fetches (biggest win, costly
reconnects/risk), lower `MIN_NAP_US` toward ~2ms, and — first — the PPK2
measurement (55mA baseline vs Stage 1 vs Stage 2b) to size all of it.

**Practical notes.** Logs written while asleep stay in the 1KB RTT buffer
until a GPIO2 press lets probe-rs attach; whole lines that don't fit are
dropped, so the battery task skips logging while `power::SLEEPING`. Don't
grow the RTT buffer: the main stack is whatever RAM is left after statics,
and the TLS handshake overflowed it (esp-hal stack-guard `Watchpoint`) with
an 8KB buffer — **under 7KB of stack headroom at ~114KB**. (Resolved 2026-10-12: the TCP TX buffer went from 32KB to 4KB, so the stack is now ~136KB. See [[Telemetry-and-Recovery]].)

## Already implemented

- **CPU clock reduced to 80 MHz** (`main.rs:72`) instead of the ~240 MHz default.
- **No BLE** — `esp-rtos`/`esp-radio` features only enable `wifi`, not `ble` ([[Build-and-CI#Cargo features]]/`Cargo.toml`).
- **Adaptive WiFi TX power** — starts at max, steps down toward a floor after enough consecutive successful connect+fetch cycles, locks at max after any failure. (Re-based from a 5s-tick/3-min-stable-connection design to a per-cycle-streak design on the `light-sleep-wifi-cycle` branch — see [[Task-Wifi-Connect]] for the current state machine and exact constants, which now live in `wifi_connect.rs` rather than being duplicated here.)
- **WiFi modem power-save owned by the fetch cycle** — superseded on the `light-sleep-wifi-cycle` branch by full radio teardown between cycles (see "In progress" above); `WIFI_PS_NONE`/`MAX_MODEM` toggling is still used briefly around each cycle's DHCP+handshake window. See [[Task-Fetch]].
- **Adaptive input poll rate** — 500ms idle, 150ms for 10s after a touch. See [[Task-Input-Read]].
- **Connection-aware serial backoff** — 300ms when a host is reading the USB frame, 2s when not; "host present" also requires the USB SOF frame counter to be advancing (2026-10-11). See [[Task-Serial-Status]].
- **No WiFi sniffer / promiscuous mode** — removed specifically because it prevented modem sleep (see `wifi.rs`'s one-line comment and git history `3fe2665`).
- **Persistent HTTPS connection across fetch cycles** (superseded on the
  `light-sleep-wifi-cycle` branch — see "In progress" above; kept here as the
  historical record of what shipped before that redesign). [[Task-Fetch]] now connects once (`client.resource(HOST_URL)`) and reuses that TLS connection for repeated 60s-interval GETs instead of doing a full DNS+TCP+TLS handshake every cycle, falling back to a fresh reconnect only when the connection is actually lost (server closed it, transport error). See [[Task-Fetch#Structure: outer connect loop, inner reuse loop]] for the design and why it degrades gracefully if the server doesn't cooperate.
- **Modem allowed to sleep (`WIFI_PS_MAX_MODEM`) for the entire life of a reused connection, not just the idle gap** (superseded on the `light-sleep-wifi-cycle` branch — see "In progress" above; kept as the historical record). **Verified on real hardware (2026-07-04, physical board over USB, `cargo run --release --features use-env`)**: 3 consecutive successful fetch cycles (200 status, full ~271 KB body streamed and parsed, 608 stations matched each time) with the modem never forced awake outside the initial connect — no dropped requests, no reconnects triggered. See [[Task-Fetch]].
- **AL5887 chip-disable during idle.** [[Task-Station-LEDs]] now calls `al5887.set_chip_enable(false)` when the selected station goes to `StationIdx::None`, instead of just zeroing all 12 LEDs' brightness/color registers — this cuts the driver chip's own quiescent draw during no-touch idle, not just LED current, and as a side effect trades ~48 SPI register writes per idle transition for 1. Re-enabled (`set_chip_enable(true)`) only when coming back out of idle into a real station selection. See [[Hardware-Drivers#AL5887 LED driver]].

## Not yet done — biggest remaining opportunities

Roughly in order of likely impact, given what's implemented above:

1. ~~**Full-network GBFS download every 60s — conditional GET.**~~ **Tested directly against the live endpoint (curl, 2026-07-04) and ruled out at the current poll rate — for two independent reasons.** First, the premise was wrong: `TARGET_STATIONS` turns out to have 611 entries (essentially the whole network, see [[Data-Model#TARGET_STATIONS]]), not the ~120 an earlier version of this vault mistakenly claimed — so there's no meaningful "extract a small fraction" saving available even in principle. Second, and independent of that: the feed supports `If-None-Match`/`If-Modified-Since` correctly (confirmed: matching ETag → `304`, 0-byte body vs. 271 KB for `200`), but it declares `"ttl": 60` and regenerates on a fixed 60-second schedule with a fresh per-station `last_reported` timestamp every time — confirmed `Last-Modified` and `ETag` both changed between two checks 60s apart. Since this firmware polls every 60s, matching the feed's own regeneration interval, a conditional GET would almost never actually return a `304` in practice. Not worth the added complexity.
2. **No CPU light-sleep.** **Now being actively pursued — see "In progress" at the top of this note** (`light-sleep-wifi-cycle` branch; working as of Stage 2b, 2026-10-11). Confirmed by grep: nothing in this codebase calls into `esp-hal`'s RTC sleep primitives (`esp_hal::rtc_cntl::sleep`, which does exist and support the S3). All the sleep here is WiFi-modem-level, not CPU-level — the CPU only idles via the Embassy executor's normal WFI between task wakeups, never enters a deeper sleep state that clock-gates more of the chip.
   - **Attempted and reverted (2026-07-26).** A custom `esp_rtos` idle hook (`src/power.rs`, `light-sleep` feature) dropped the CPU into `rtc.sleep_light()` in fixed 20ms chunks whenever idle. On real hardware this made the board unresponsive to probe-rs/espflash over the native USB-Serial-JTAG interface — recovered only by manually strapping GPIO0 low at reset to force ROM download mode. Root cause, confirmed at the hardware-register level (not just the missing wake source): ESP32-S3 has no USB wake bit in its `WakeTriggers` register at all — no software fix was possible as originally attempted. Stage 2's design works around this by registering the provisioning button (GPIO2, RTC-capable) as a physical `Ext0WakeupSource` escape hatch instead, with GPIO0-strap recovery as the backstop.
3. **Fixed LED brightness (30/255) regardless of how many LEDs are lit.** No global dimming based on total lit-LED count, even though `LedGlobalDimming` register exists unused ([[Hardware-Drivers#AL5887 LED driver]]).

## Smaller/secondary ideas

- `listen_interval=10` (~1s modem sleep) in [[Task-Wifi-Connect]] could potentially go deeper now that the fetch cycle handles `PS_NONE` around the fragile DNS/TLS window — worth re-testing empirically.
- ~~`battery_task` samples every 5s~~ — done: now every 30s (commit `80962ad`, [[Task-Battery]]).
- `input_read_task` polls on a fixed timer even at idle; if the shift-register hardware exposes any kind of change/interrupt line, interrupt-driven reads would remove the periodic wakeup entirely rather than just slowing it to 500ms.
- `station_leds_task`'s unconditional `Timer::after(50ms)` every loop iteration ([[Task-Station-LEDs]]) is an extra ~20Hz wakeup layered on top of the input task's own cadence.
- `dprintln!` resolves to `rtt_target::rprintln!` in normal (non-`debug-serial`) builds too ([[Build-and-CI#Cargo features]], `lib.rs`), including in per-cycle hot paths (wifi maintenance loop, battery task, fetch task) — cheap per call, but it's live in shipped builds, not compiled out.
- `serial_status_task` runs unconditionally even if no USB host will ever be attached in a standalone battery deployment; without a VBUS-presence check there's no way to skip spawning it entirely.
- Release profile uses `opt-level = 's'` (size) rather than `3` (speed) — see [[Build-and-CI#Profiles]]. Worth an A/B current-draw measurement, since fewer CPU cycles per task wakeup (shorter active windows) is a distinct axis from binary size.

## What a light-sleep design would have to account for

If CPU light-sleep is ever pursued (item 2 above), the things currently keeping the CPU "busy enough to matter" are: the six always-running tasks' timers (ranging 50ms–60s), two blocking SPI buses (100kHz, so each transfer takes real wall-clock time with the CPU parked in a blocking call, not free to sleep), and the WiFi/USB peripherals needing their own clocks active regardless of CPU sleep state. Aligning the task wakeup periods to common multiples (see [[Architecture#Task graph]]) would be a low-risk first step that makes any future light-sleep work more effective, independent of whether light-sleep itself gets implemented.
