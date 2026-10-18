---
tags: [power, plan]
---

# Sleep floor: handoff and plan (2026-10-18)

Starting point for the next round of power work. Background: [[Power-Management]],
full experiment record: [[Power-Optimization-Log]].

## Where things stand (v1.4.0, on `main`)

Average current with a 120s fetch gap, PPK2 at 4.5V in place of the AA pack:

| Part | Average | Notes |
|---|---|---|
| **Sleep floor (board + chip)** | **4.16mA** | ~90% of the total; the target of this plan |
| Fetch cycle, clean | 0.35mA | ~45mA·s per cycle, ~190ms radio |
| Fetch cycle, bad XB8 phase | ~2.7mA | ~320mA·s per cycle; the gateway ignores the board's replies 1-3s after reconnecting. Not fixable in firmware (sniffer proof in the log) |
| Input poll (500ms) | 0.07mA | 2 wakes/s, ~1.2ms and ~34mA·ms each |
| Serial status, battery, other | ~0 | share wakes with the input poll |
| **Total** | **4.6 (clean) - 6.9 (bad) mA** | |

Battery life on 3×AA (alkaline ~2,100mAh usable to the 1.11V/cell cutoff;
lithium Li-FeS₂ ~3,100mAh, both estimates):

| Fetch every | Avg mA (clean / bad) | Alkaline days | Lithium days |
|---|---|---|---|
| 1 min | 4.98 / 9.39 | 18 / 9 | 26 / 14 |
| 2 min (now) | 4.60 / 6.85 | 19 / 13 | 28 / 19 |
| 3 min | 4.48 / 5.99 | 20 / 15 | 29 / 22 |
| 5 min | 4.38 / 5.29 | 20 / 17 | 29 / 24 |

Model: 4.23mA fixed + charge per cycle ÷ interval. The floor caps the board at
~21 days (alkaline) / ~31 days (lithium) even with no fetches, so **the floor
is the only big lever left**.

## What the floor is made of (estimate, to be measured)

- **Board, ESP32 excluded: ~1.8mA.** Measured on v1.3.1 with the ESP32 held in
  reset (pins floating, so only approximate). Candidates: regulator quiescent
  current (part unknown), AL5887 LED driver standby (chip-disabled while idle),
  battery divider resistors, pull-ups, leakage.
- **ESP32-S3 in light sleep: ~2.3mA.** The datasheet's light-sleep figure is
  ~0.24mA, so ~1.5-2mA of room on the chip side.

Light-sleep config today (`src/power.rs`, `sleep_config()`): ESP-IDF's
light-sleep values, 8MHz oscillator and XTAL off, digital rail dropped
(`dig_dbias_slp` 5, `dbg_atten_slp` 5), bias and current monitor powered down.
Powering down the modem domain too saved only ~0.1mA (round 1).

## Plan

### Step 1: measure the split (firmware only)

1. **Deep-sleep test build:** a throwaway binary (or feature) that configures
   all pins to a defined low-leakage state and calls deep sleep at once. The
   ESP32 then draws ~10µA, so the PPK2 reading is the board alone. More
   reliable than the v1.3.1 "held in reset" number.
2. **Light-sleep options, one PPK2 run each** (each a few minutes; the floor
   shows directly in the 10s progress averages between fetches):
   - **GPIO states during sleep**: the likeliest hidden drain. Check every pin:
     LED driver SPI/enable lines, shift-register lines, board-ID straps
     (GPIO37/38 pulled up), GPIO2 wake button (RTC pull-up), the battery ADC
     pin. Drive or isolate so nothing fights a pull-up or floats into a CMOS
     input. ESP-IDF does this with `gpio_sleep_sel`/hold; esp-hal may need
     direct register writes.
   - **Flash power-down in sleep (VDD_SDIO off)**: needs a flash power-up
     delay on wake; check esp-hal's `RtcSleepConfig` for `vddsdio_pd_en`.
   - **RTC peripherals off**: needs the GPIO2 escape-hatch wake (`Ext0`) moved
     to a source that doesn't need the RTC peripheral domain.
   - CPU power-down in light sleep (if esp-hal supports retention on the S3).

### Step 2: board level (needs the schematic / part numbers)

- Regulator: its quiescent current may be most of the 1.8mA. Options: a
  low-Iq LDO (single-digit µA), or a buck converter, which would also cut every
  radio burst by ~25% at 4.5V in.
- AL5887 standby current with chip-enable low; if large, a MOSFET on its supply.
- Battery divider values (10k/10k wastes ~0.2mA; 1M/1M ~2µA, may need a cap for
  the ADC).

### Step 3 (only if 1-2 leave a lot): deep sleep between fetches

- Could take the whole board under ~1-2mA, depending on step 2.
- The touch grid is polled through shift registers every 500ms with no
  interrupt line, so deep sleep needs the **ULP coprocessor** (ESP32-S3
  ULP-RISC-V) to poll it and wake the main CPU on a touch.
- Every wake is a reboot plus a cold WiFi start: ~1.8s of extra radio per cycle
  (round 19: de-initialising WiFi each cycle took the average from ~11 to
  ~14mA). Only pays off with long fetch gaps.

## How to measure (workflow that worked)

- **PPK2**: `tools/ppk2/ppk2.py` (uv script) in source-meter mode at 4.5V,
  batteries out. `measure -s N --csv run.csv --json run.json --progress
  run.progress` streams 1ms bins; the CSV is appended every 10s, so a run can
  be analysed while it is still going or after it is stopped. `windows`
  summarizes fixed windows; `tools/ppk2/breakdown.py run.csv` splits floor /
  fetch cycles / wakes. Stopping a `measure` process cuts the board's power;
  starting one powers it on (a fresh boot).
- **Flashing**: USB VBUS powers the board, so PPK2-only power is impossible
  while USB is plugged in. Sequence: `ppk2.py off`, plug USB, flash within the
  20s no-sleep boot window (`probe-rs download --chip esp32s3 --preverify
  ELF`, retry if the probe isn't ready), unplug, then `measure` (powers on).
  A board that has light-slept on USB enumerates but doesn't answer; replug.
  Toolchain: source `~/export-esp.sh` in bash (the user's shell is fish).
- **probe-rs RTT**: `probe-rs attach --chip esp32s3 ELF` works whenever the
  CPU isn't light-sleeping (boot window, or while the radio is on).
- **Telemetry**: every cycle posts to the HA add-on (`http://10.0.0.107:8099`,
  `/api/reports?limit=N`). With `sleep-trace`, `task_polls_total` gives exact
  per-task wake counts (order: main, net, battery, LEDs, fetch, input, serial)
  and `naps_total`/`near_deadline_waits_total` the nap counts.
- **Builds**: `cargo build --release --features use-env,log-server,gbfs-mirror,sleep-trace`;
  `FETCH_GAP_S=15` for faster cycles while iterating.
- **Pushing**: the pre-push hook runs `cargo build --release`, which needs
  `export-esp.sh` sourced in the same shell. A `v*` tag push publishes a
  GitHub release.

## Cautions

- The Xfinity XB8 refused the board for ~20 minutes after hundreds of
  reconnects every ~18s (2026-10-18). Keep short-gap test runs short; a board
  stuck at ~200mA with "Retrying connect..." in RTT is the gateway, not
  firmware (identical earlier firmware failed the same way).
- Don't drop the WiFi controller between cycles without silencing embassy-net
  first (round 19 crash: TX into a de-initialised driver).
- The idle hook's TIMG1 tick rate must decode the prescaler like esp-hal
  (DIVIDER 1 or 2 = /2); that bug cost ~4.7mA until v1.4.0.
