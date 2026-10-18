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

#### Step 1 prepared (2026-10-18, branch `worktree-sleep-floor-step1`)

**Finding while preparing:** the AL5887 is never chip-disabled after boot
until the first touch-and-release. The input task's first reading is
`StationIdx::None`, which equals the LED task's starting `last_station`, so the
idle branch (`set_chip_enable(false)`) never runs. Every PPK2 floor so far
(nobody touches the board) had the driver in active mode with all LEDs off.
This may be a large part of the "board" 1.8mA.

Build knobs (env vars at build time, all default to today's behaviour):

| Knob | Values | Where |
|---|---|---|
| `DEEP_SLEEP_TEST` | 1: board pins at idle levels, held, then deep sleep (AL5887 standby). 2: same, with LED_EN low (AL5887 shutdown) | `src/deep_sleep_test.rs` |
| `LED_IDLE` | 1: chip-disable from boot too (the fix). 2: also LED_EN low while idle (hardware shutdown, re-init on touch) | `src/tasks/station_leds.rs` |
| `SLEEP_VDDSDIO_PD` | 1: flash supply off in light sleep. No IRAM flash-ready wait on wake, so it may crash; check the reset cause | `src/power.rs` |
| `SLEEP_RTC_PERI_PD` | 1: RTC peripherals off in light sleep; GPIO2 wake becomes a digital GPIO wake. Check that the battery mV still reads correctly | `src/power.rs` |

`tools/ppk2/build-step1.sh [name...]` builds every variant to
`target/step1/<name>.elf` (`--features use-env,log-server,gbfs-mirror,sleep-trace`).
Run order, one PPK2 run each (a few minutes; read the floor between fetches):

1. `baseline`: today's firmware, reference floor.
2. `led-idle-1`: AL5887 chip-disabled from boot. Delta = the boot-idle bug.
3. `led-idle-2`: AL5887 in hardware shutdown. Delta vs 2 = standby vs shutdown.
4. `deep-al5887-standby`: board alone (ESP32 ~10µA) with the AL5887 in
   standby, i.e. the same board state as run 2. Floor(2) − this = ESP32 share.
5. `deep-al5887-shutdown`: board alone with the AL5887 shut down. Whatever is
   left is the regulator, dividers, pull-ups and leakage, the input to step 2.
6. `vddsdio-pd` and `rtc-peri-pd`: each on top of `LED_IDLE=1`, so compare
   with run 2.

Deep-sleep builds stay awake 20s after reset (attach/flash window), then sleep
until a GPIO2 press, which boots into the same window again. Pad holds survive
a deep-sleep wake but not a power cycle. A fresh `measure` power-cycles.

#### Step 1 results (2026-10-18, PPK2 at 4.5V)

Floor = `breakdown.py` sleep floor. "Cable" = the data-only USB cable (VBUS
cut) left plugged in so probe-rs can flash while the PPK2 powers the board; it
adds ~0.17mA (D+ pull-up into the host).

| Run | Cable | Floor | Notes |
|---|---|---|---|
| `baseline` (v1.4.0) | out | **4.14mA** | matches the 4.16mA above |
| `led-idle-1` (AL5887 chip-disabled from boot) | out | **2.64mA** | −1.50mA |
| `led-idle-1` | in | 2.81mA | cable +0.17mA |
| `led-idle-2` (+ LED_EN low) | in | 2.82mA | no gain over chip-disable |
| `power-save` (`AL5887_POWER_SAVE=1`, chip left enabled) | in | 2.81mA | same as chip-disable |
| `deep-al5887-standby` (board alone) | in | **1.53mA** | ESP32 in deep sleep |
| `deep-al5887-shutdown` (board alone, LED_EN low) | in | 1.52mA | AL5887 standby is ~15µA |
| `vddsdio-pd` (+ `LED_IDLE=1`) | in | 2.83mA | no gain (S3FN8 in-package flash) |
| `rtc-peri-pd` (+ `LED_IDLE=1`) | in | n/a | **hangs** at the first nap (~47mA, JTAG unreachable), RTC watchdog reset after 60s |

Why the AL5887 fix works: DEVICE_CONFIG1 resets to 0xAE, which has
**Power_Save_EN (bit 4) off** (TI's LP50xx defaults it on). The firmware never
wrote DEVICE_CONFIG1, so a chip-enabled AL5887 with all LEDs off stayed in
normal mode (~1.5mA) instead of the datasheet's 15µA power-saving mode. Either
fix works: chip-disable when idle (`LED_IDLE=1`) or set Power_Save_EN at init
(`AL5887_POWER_SAVE=1`). LED touch/release/re-touch verified by hand on
`led-idle-2`.

**Now the default:** `init_driver` always sets Power_Save_EN, and `LED_IDLE`
defaults to 1 (the `AL5887_POWER_SAVE` knob is gone). Verified on the PPK2:
floor 2.83mA with the cable in (≈2.64mA without). `build-step1.sh`'s
`baseline` is now this build; the v1.4.0 run is kept as
`target/step1/v140-baseline.*`.

New split (cable out, estimated): floor ~2.64mA = **board ~1.35-1.5mA**
(regulator, battery divider, pull-ups, leakage; the deep-sleep reading includes
some of the cable's 0.17mA) + **ESP32-S3 light sleep ~1.1-1.3mA** (datasheet
~0.24mA). Step 2 (board) is now at least as big a lever as the chip.

**Board-side follow-up (same day).** Known parts explain only ~0.1mA of the
board's ~1.5mA: LDO quiescent tens of µA (per the user), battery divider 122kΩ
total (37µA at 4.5V), AL5887 ~15µA, ESP32 deep sleep ~10-20µA. No power LED.
- Hand test (deep sleep, 2s averages): touching the five 74HC165s' pins
  changed nothing (1.607-1.612mA throughout), so floating shift-register
  inputs are unlikely to be the drain.
- `DEEP_SLEEP_TEST=3` (`deep-hiz`, every board pin high-impedance): 1.62-1.63mA,
  the same as with the pins driven (1.61mA). No ESP32 pin is pushing current.
- So the remaining ~1.4mA is hardware outside the ESP32's pins. Next: power
  the 3.3V rail directly from the PPK2 (LDO bypassed) to split LDO / battery
  side from the 3.3V parts.

Not done: `rtc-peri-pd` hang root cause (likely the timer or GPIO wake path
with the RTC peripheral domain off in esp-hal 1.0; not worth chasing before
step 2 / an esp-hal upgrade).

Flashing over the data-only cable: `ppk2.py on`/`cycle` did not bring up the
probe; powering through a short `measure -s 30` did, every time (flash in its
first 20s).

Dropped from step 1: **CPU power-down**. esp-hal 1.0 (ours) and the latest
release, 1.2.0, have no CPU retention. `main` gained it after 1.2.0 (#6288,
2026-09-30, S3 backend using the retention DMA, ~8.8KB buffer via
`LowPower::install_cpu_retention_memory`). Using it means moving to an
unreleased esp-hal with the new `LowPower` sleep API and matching
esp-rtos/esp-radio (our esp-rtos patch included). 1.2 also brings automatic
light sleep for the S3, which could replace much of `power.rs`. ESP-IDF quotes
~650µA saved on the C3; there is no S3 figure. Worth its own step after 1-2.

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
