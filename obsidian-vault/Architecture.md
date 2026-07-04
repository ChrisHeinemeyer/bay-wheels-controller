---
tags: [architecture]
---
	
# Architecture

## Hardware summary

- **MCU**: ESP32-S3, clocked at 80 MHz (`esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz)`, `src/bin/main.rs:72`) instead of the default ~240 MHz.
- **Power**: 3× AA alkaline through a resistive divider into an ADC pin — see [[Task-Battery]]. This is a battery product, not mains-powered, which is why [[Power-Management]] matters.
- **Display**: an AL5887 RGB LED driver chip over a bit-banged blocking SPI bus (SPI3, 100 kHz), driving 12 individually addressable LEDs. See [[Hardware-Drivers#AL5887 LED driver]].
- **Input**: a shift-register-based touch/button grid read over a second bit-banged SPI bus (SPI2, 100 kHz). See [[Hardware-Drivers#Shift-register input grid]].
- **Board identity**: a 2-bit hardware strap on GPIO37/GPIO38 (both pulled up internally; a populated resistor to GND grounds a bit) read once at boot to determine `BoardId`, which selects which `(row, col) → station` map applies. See [[Data-Model#BoardId and BOARD_STATION_MAP]].
- **USB**: USB-Serial-JTAG, multiplexed between three uses depending on boot path/feature: RTT-less debug logging (`debug-serial` feature), first-boot WiFi provisioning wizard, or the binary status frame in normal operation. Never more than one owner at a time.

## Boot sequence (`src/bin/main.rs`)

1. `esp_hal::init` with 80 MHz clock, then a 64 KiB heap allocator (`esp_alloc::heap_allocator!`).
2. Read GPIO2 (provisioning button) *before* Embassy starts. If held low at boot → jump straight into `provisioning::run_provisioning` (blocking, never returns; see [[Provisioning-and-Config]]).
3. Otherwise, try to load WiFi credentials from NVS flash (`wifi_config::load_credentials`). If none exist, also fall into provisioning mode automatically — first boot after a fresh flash always ends up here.
4. Start the Embassy executor (`esp_rtos::start`) and initialize the WiFi/BLE radio (`esp_radio::init`, BLE feature not enabled — see [[Build-and-CI#Cargo features]]).
5. Spawn `wifi_connect_task` (owns the `WifiController` for the rest of the program) and `net_task` (drives the `embassy_net::Runner`).
6. Read the board-ID strap (GPIO37/38), stash it in `STATUS`.
7. Construct the ADC (battery), the shift-register SPI device, and the AL5887 SPI device — these grab their GPIOs/SPI peripherals here so ownership can move into their tasks.
8. If not `debug-serial`, spawn `serial_status_task` last, after everything else, so USB-Serial-JTAG is free for it (it's shared with the debug logger and provisioning wizard, but never used by more than one at a time — the `#[cfg]` gates enforce this at compile time, not runtime).
9. Spawn `battery_task`, `station_leds_task`, `fetch_task`, `input_read_task`.
10. Main task parks forever in a 60s sleep loop — it's only there because `#[esp_rtos::main]` requires an `async fn ... -> !`.

## Task graph

Six long-running tasks plus the network runner, all spawned once and never joined:

| Task | File | Cadence | Owns |
|---|---|---|---|
| `wifi_connect_task` | [[Task-Wifi-Connect]] | scan+connect once, then 5s maintenance loop | `WifiController` |
| `net_task` | `src/bin/main.rs:266` | driven by embassy-net internally | `embassy_net::Runner` |
| `fetch_task` | [[Task-Fetch]] | 60s | the `Stack` reference, TCP/TLS buffers |
| `station_leds_task` | [[Task-Station-LEDs]] | reactive to `STATION_SIGNAL`, +50ms tick | `Al5887` (SPI3) |
| `input_read_task` | [[Task-Input-Read]] | 150ms (active) / 500ms (idle) | `ShiftRegister` (SPI2) |
| `battery_task` | [[Task-Battery]] | 5s | `Adc`/`AdcPin` |
| `serial_status_task` | [[Task-Serial-Status]] | 300ms (connected) / 2s (disconnected) | `UsbSerialJtag` (async) |

None of these tasks call each other or hold a reference to another task's peripheral. They coordinate purely through the statics in [[Shared-State]] (`STATUS` mutex, `STATION_SIGNAL`, `STATION_DATA_SIGNAL`, `FETCH_SIGNAL`). This is a clean, fully-decoupled Embassy design — you can reason about each task in isolation as long as you know what it reads/writes in `signals.rs`.

## Data flow, end to end

```
GBFS HTTPS feed (~600 stations, all of Bay Wheels)
  → fetch_task (fetch.rs) — 60s poll, streams response
    → StreamingStationParser (station_parser.rs) — filters down to TARGET_STATIONS (611, ~all of them)
      → STATION_DATA_SIGNAL  +  FETCH_SIGNAL (timestamp)
        → station_leds_task caches the [StationData; 610] array

touch grid (shift register)
  → input_read_task (input_read.rs) — polls every 150–500ms
    → value_to_station() looks up (row, col) in BOARD_STATION_MAP for this board's BoardId
      → STATION_SIGNAL (StationIdx)
        → station_leds_task looks up bike counts for that station, drives the AL5887

STATUS mutex (battery %, wifi state, rssi, tx power, last input, led_states, ...)
  → serial_status_task packs it into a 51-byte binary frame every 300ms/2s
    → USB-Serial-JTAG → companion web app (web/) renders it
```

See [[Data-Model]] for how `StationIdx` ties the touch grid, the GBFS feed, and the web UI's map together, and [[Companion-Tools]] for how that mapping is authored in the first place.
