---
tags: [architecture, concurrency]
---

# Shared State (`src/tasks/signals.rs`)

All inter-task communication goes through statics defined in this one file. There is no message passing / channels — just `embassy_sync::signal::Signal` (latest-value, overwrite-on-signal, single reader) and one `embassy_sync::mutex::Mutex` guarding a plain struct. Everything uses `CriticalSectionRawMutex`, i.e. these are safe across the single-core executor via critical sections, not real multi-core locking.

## `STATION_SIGNAL: Signal<StationIdx>`
- Written by [[Task-Input-Read]] on **every** poll cycle (150–500ms), even when the reading hasn't changed.
- Read by [[Task-Station-LEDs]], which is structurally built around `STATION_SIGNAL.wait()` — this signal is effectively what drives that task's loop cadence.

## `STATION_DATA_SIGNAL: Signal<[StationData; STATION_DATA_LEN]>`
- Written once per successful GBFS fetch by [[Task-Fetch]] (`STATION_DATA_SIGNAL.signal(station_data)`, `fetch.rs:121`) — carries the *entire* 610-entry array, not a diff.
- Read (via `try_take`, non-blocking) by [[Task-Station-LEDs]] whenever the selected station changes, so it always has the latest snapshot available without blocking on it.
- Also used at startup: `station_leds_task` blocks on this via `wait_with_breathing` until the very first fetch completes, before it does anything else (see [[Task-Station-LEDs#Startup breathing]]).

## `FETCH_SIGNAL: Signal<Instant>`
- Written by [[Task-Fetch]] at the end of every successful fetch — carries the fetch's completion timestamp, used purely to compute "how stale is this data" for the wifi-status LED (see [[Task-Station-LEDs#Fetch-age indicator]]).

## `STATUS: Mutex<SystemStatus>`
The catch-all shared state, read/written by nearly every task:

| Field | Written by | Read by |
|---|---|---|
| `board_id` | `main.rs` once at boot | [[Task-Serial-Status]] |
| `battery_pct` | [[Task-Battery]] | [[Task-Serial-Status]] |
| `wifi_connected`, `rssi` | [[Task-Wifi-Connect]] | [[Task-Serial-Status]] |
| `tx_power` | [[Task-Wifi-Connect]] | [[Task-Serial-Status]] |
| `last_fetch_at` | [[Task-Fetch]] | [[Task-Serial-Status]] (fetch-age field in the frame) |
| `station_input`, `station_input_row/col` | [[Task-Input-Read]] | [[Task-Serial-Status]] |
| `led_states` | [[Task-Station-LEDs]] | [[Task-Serial-Status]] (mirrors what's actually on the physical LEDs, for the web UI to render a virtual board) |

Note this is a single global lock guarding an unrelated grab-bag of fields — fine at this scale (a handful of tasks, cheap critical sections, no contention), but worth knowing if the struct ever grows: every writer briefly blocks every other reader/writer, including the frequently-polled `serial_status_task`.

## Design note

This is the classic Embassy "shared statics + signals" pattern rather than channels/actors. It works well here because:
- Every signal is genuinely "latest value wins" (station selection, station data snapshot, fetch timestamp) — nothing needs a queue.
- There's no backpressure concern since consumers poll or `wait()` and always want the newest value.

The tradeoff is that adding a new task that needs to react to *every* fetch (not just the latest) wouldn't fit this model without changes — `Signal` drops values that aren't consumed before the next `.signal()` call.
