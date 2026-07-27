---
tags: [task, display]
---

# `station_leds_task` (`src/tasks/station_leds.rs`)

Owns the [[Hardware-Drivers#AL5887 LED driver|AL5887]] and is the only task that writes to it. Translates "which station is selected" + "what's that station's live data" into 12 physical RGB LEDs.

## Startup breathing

Before doing anything else, it initializes the driver (`al5887.init_driver()`) and then blocks — via `wait_with_breathing` — first on `STATION_DATA_SIGNAL`, then on `FETCH_SIGNAL`, i.e. until the very first successful GBFS fetch completes. While waiting on each, it breathes `WIFI_STATUS_LED` (`Led11`) in a soft blue (`WIFI_STATUS_BREATHE_COLOR`) using a triangle-wave brightness ramp, updated every `BREATH_TICK = 30ms`, stepping phase by `BREATH_STEP = 4` (full cycle ≈ 256/4 × 30ms ≈ 1.9s). This is a recent addition (see git history: `9e98d11 leds: breathe wifi-status LED during first connect, drop boot-time LED0 flash`) that replaced an earlier boot-time LED flash.

## Main loop

```
loop {
    station = STATION_SIGNAL.wait().await        // blocks until input_read_task signals
    if let Some(t) = FETCH_SIGNAL.try_take() { fetch_time = t }   // non-blocking freshen
    fetch_age = now - fetch_time
    if station != last_station {                 // only touches SPI on actual change
        maybe pull fresh STATION_DATA_SIGNAL.try_take()
        if station == None: chip_enable(false)    // blanks display + cuts driver quiescent draw
        else: chip_enable(true) if coming from idle, compute LEDs, write via SPI
    }
    Timer::after(50ms)
}
```

Because [[Task-Input-Read]] signals `STATION_SIGNAL` on *every* poll (not just on change), this loop wakes roughly as often as the input task polls (150–500ms) even when nothing visually changes — the `station != last_station` guard is what prevents redundant SPI writes, not the wait itself.

## LED layout (`get_leds`)

Per selected station, up to `MAX_LEDS = 12` LEDs are lit:
- **Station has no bikes at all** (`num_ebikes_available == 0 && num_bikes_available == 0`): 1 red LED (`STATION_EMPTY_LEDS = [Led6]`).
- **Otherwise**: up to 5 green LEDs for ebikes (`EBIKE_LEDS = [Led1..Led5]`) and up to 5 blue LEDs for mechanical bikes (`MECHANICAL_BIKE_LEDS = [Led10, Led9, Led8, Led7, Led0]`, deliberately reverse-ish ordering — likely matches the physical PCB layout radiating outward from the station's grid position). Counts above 5 are silently clamped (`min(count, EBIKE_LEDS.len())`).
- **Always**, when a station is selected: 1 LED (`Led11`, same physical LED as the boot-time wifi-breathe indicator) shows fetch freshness — green (<60s old), yellow (<180s), red (≥180s) via `get_fetch_age_leds`. So `Led11` does double duty: "connecting" breathe indicator before first data, "how stale is this data" indicator after.

All lit LEDs are driven at a flat brightness of `30` (`al5887.rs:153`, `set_vec_led`) — not user- or ambient-configurable currently.

## Mirroring to `STATUS.led_states`

Every time LEDs are actually written, the same `(r, g, b)` values are also written into `STATUS.led_states[led_idx]` so [[Task-Serial-Status]] can report the true current display state to the companion web UI (which renders a virtual copy of the board) without needing to read the SPI bus itself.
