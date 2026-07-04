---
tags: [task, parsing]
---

# `StreamingStationParser` (`src/tasks/station_parser.rs`)

Not an Embassy task itself — a stateful incremental parser instantiated fresh each fetch cycle inside [[Task-Fetch]], fed one HTTP chunk (1 KiB) at a time via `process_chunk`. Exists so the firmware never has to buffer the full GBFS response (which covers the whole Bay Wheels network) in RAM.

## Phases (`ParserPhase`)

1. `SearchingForStations` — scans incoming text for the literal `"stations"` key, then the following `[`. Everything before that is discarded (only the last ~100 bytes are kept as `remainder` in case the key straddles a chunk boundary).
2. `InStationsArray` — once inside the array, repeatedly: skip whitespace/commas, find a `{`, find its matching `}` via `find_matching_brace` (a hand-rolled brace-depth/string-aware scanner — no picojson streaming used here, just a byte scan to find object boundaries), then hand that single object's JSON slice to `picojson::SliceParser` to pull out `station_id`, `num_bikes_available`, `num_ebikes_available`.
3. `Done` — hit the closing `]`.

## Buffering

- `remainder: String<8192>` — carries an incomplete station object across chunk boundaries. Sized to hold one full station JSON object.
- Each `process_chunk` call concatenates `remainder + chunk` into a scratch `String<12288>` (8 KiB + 4 KiB headroom) before scanning.

## Filtering

`parse_single_station` only returns `Some(StationData)` if the parsed `station_id` UUID matches an entry in `target_stations: &'static [(&str, StationIdx)]` (i.e. [[Data-Model#TARGET_STATIONS]], ~120 entries out of the full network). Everything else increments `ignored_count` and is dropped — this is where the "only care about ~120 of 600+ stations" filtering actually happens, station-by-station, as the response streams in. It does **not** reduce what's downloaded over the air (see [[Task-Fetch#Cost per cycle]]) — it only avoids buffering/processing the non-target stations' data further.

## A quirk worth knowing (`num_bikes_available` computation)

```rust
num_bikes_available: num_bikes_available.unwrap_or(0) - num_ebikes_available.unwrap_or(0),
```

GBFS's `num_bikes_available` is the *total* (mechanical + electric), so this line subtracts out ebikes to get a mechanical-only count for the LED display (mechanical and ebike counts are shown with different colors — see [[Task-Station-LEDs]]). If a malformed/edge-case feed ever reported `num_ebikes_available > num_bikes_available`, this is a `u8` subtraction with `overflow-checks = false` in the release profile ([[Build-and-CI#Profiles]]) — it would silently wrap instead of panicking. Downstream, `get_leds` in `station_leds.rs` clamps with `min(..., EBIKE_LEDS.len())` before indexing, so a wrapped huge value wouldn't crash, just render a maxed-out LED row. See [[Open-Questions]].
