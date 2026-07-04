---
tags: [task, input]
---

# `input_read_task` (`src/tasks/input_read.rs`)

Polls the [[Hardware-Drivers#Shift-register input grid|shift register]] to find which (row, column) cell is currently active (a physical touch/button on the board), maps it to a `StationIdx` for this board's `BoardId`, and publishes it.

## Adaptive polling

```rust
const POLL_ACTIVE_MS: u64 = 150;     // fast poll right after a touch
const POLL_IDLE_MS: u64 = 500;       // normal idle poll
const ACTIVE_WINDOW_MS: u64 = 10_000; // how long "fast" persists after last touch
```

Every read that resolves to a non-`None` station stamps `last_touch = Some(Instant::now())`. As long as `now - last_touch < 10s`, the loop polls at 150ms; otherwise it backs off to 500ms. This means a user actively browsing stations gets snappy feedback, while a board sitting untouched only wakes the SPI bus twice a second. This was added per git history (`3a52045 input: adaptive poll rate — 500 ms idle, 150 ms after touch`) as a deliberate battery-life tradeoff — see [[Power-Management]].

## Every poll, regardless of value

Unlike a typical debounced input handler, this task signals `STATION_SIGNAL` (see [[Shared-State]]) and writes `STATUS.station_input*` on **every** poll cycle, even if the reading is identical to last time. Downstream, [[Task-Station-LEDs]] is the one that dedupes via `station != last_station`. This means [[Task-Station-LEDs]]'s wake cadence is coupled to this task's poll cadence (150–500ms), not purely event-driven.

## Mapping (`value_to_station`)

```
(row, col) from shift register
  → if row == IDLE || col == IDLE → StationIdx::None
  → look up BOARD_STATION_MAP for this board_id
      → found → that StationIdx
      → not found → StationIdx::Unknown   (distinct from None — see Data-Model)
```

See [[Data-Model#BoardId and BOARD_STATION_MAP]] for how the map itself is built (per-board, generated from a companion mapping tool — [[Companion-Tools]]).
