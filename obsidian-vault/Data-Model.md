---
tags: [data-model]
---

# Data Model (`src/stations.rs`, `src/grid.rs`)

## `StationIdx`

A large generated-looking enum (`src/stations.rs`), one variant per Bay Wheels station across the whole network, ordinal 0 through 610 (`SouthLake`), plus two sentinels:

- `Unknown = 65534` — input was active but doesn't map to any entry in `BOARD_STATION_MAP` for the current board.
- `None = 65535` — no input active (idle grid).

`STATION_DATA_LEN = 610` sizes the `[StationData; STATION_DATA_LEN]` array that [[Task-Fetch]] populates and [[Task-Station-LEDs]] indexes directly by `station_idx as usize` — an O(1) lookup traded for a fairly large static array (610 × `size_of::<StationData>()`, each `StationData` being a `StationIdx` (2 bytes) + 2×`u8`, so ~4-8 bytes with padding — a few KB total, not huge, but notably it's sized to the *entire network*, not just the ~120 stations this device cares about, because ordinals need to be stable/unique across the whole enum).

## `Row` / `Column` (`grid.rs`)

Thin newtypes (`Row(pub u8)`, `Column(pub u8)`) over the raw shift-register bit index, each with an `IDLE = Self(0xFF)` sentinel. Exist mainly for type safety (can't accidentally compare a row to a column) and to carry the `IDLE` sentinel semantics explicitly rather than using a bare `Option<u8>` everywhere.

## `BoardId` and `BOARD_STATION_MAP`

`BoardId` (`signals.rs`) is a 2-bit hardware-strapped identifier (see [[Hardware-Drivers#Board-ID straps]]). `BOARD_STATION_MAP` (`stations.rs`) is a `&[(BoardId, &[((Row, Column), StationIdx)])]` — effectively one lookup table per physical board layout, each entry saying "at this touch grid cell, show this station." Only `Board3` (36 cells, 6×6) and `Board2` (16 cells, 1×16 — a single-row layout) have entries populated today; `Board0`/`Board1` are stubbed out as commented-out placeholders (`stations.rs:699-700`), meaning any board that straps to `0b00` or `0b01` today will resolve every input to `StationIdx::None` regardless of what's pressed — worth checking before assuming all 4 possible strap values are "supported."

[[Task-Input-Read]]'s `value_to_station` does a linear `.find()` over the current board's slice on every single poll (every 150–500ms) — fine at 16-36 entries, but note it's O(n) per lookup, not indexed.

## `TARGET_STATIONS`

`&[(&str, StationIdx)]`, ~120 entries, mapping GBFS station UUIDs (the opaque IDs Lyft's API uses, e.g. `"bfb90ed7-6039-4c61-9b13-fb60b1786dde"`) to `StationIdx`. This is the *other* direction from `BOARD_STATION_MAP` — it's what [[Task-Station-Parser]] uses to decide which stations in the GBFS feed are worth keeping. Two ID formats appear in the data: UUID-style (`"bfb90ed7-..."`) and long numeric strings (`"1838251762103669212"`) — Lyft's GBFS feed apparently mixes both ID schemes across its station list, so the matching code just does a plain string comparison rather than assuming one format.

## How the two maps relate

A given `StationIdx` variant (e.g. `ArguelloEdward`) typically appears in **both** tables: once in `BOARD_STATION_MAP` (which physical button shows it) and once in `TARGET_STATIONS` (which GBFS UUID feeds its live data). They're maintained by hand/generated separately — nothing in the type system enforces that every station reachable via touch also has a `TARGET_STATIONS` entry (if it didn't, `station_data[idx]` would just stay at its `Default` — 0 bikes, `StationIdx::None` — forever, rendering as "empty station"). See [[Companion-Tools]] for the tool that helps author `BOARD_STATION_MAP`.

## Generated artifact: `web/src/generated/station-ids.ts`

`build.rs` (see [[Build-and-CI#Codegen]]) parses `TARGET_STATIONS` out of `stations.rs` source text (a small hand-rolled parser, not using `syn`) and emits a TS file mapping ordinal → UUID, so the web UI can label stations without duplicating the list by hand. This is a one-way, build-time-only sync — editing `stations.rs` and rebuilding firmware regenerates the TS file; editing the TS file directly does nothing (it's overwritten, with a comment saying so).
