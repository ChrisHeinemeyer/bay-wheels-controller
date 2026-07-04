---
tags: [protocol, serial]
---

# Binary Serial Status Protocol

Sent by [[Task-Serial-Status]] over USB-Serial-JTAG, consumed by `web/src/serial-frame.ts` in [[Companion-Tools]]. Two frame types share the wire, distinguished by their first (magic) byte, both terminated by an XOR checksum for resync (see [[Task-Serial-Status#Framing / resync]]).

## Status frame — 51 bytes, magic `0xAB`

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | magic | `0xAB` |
| 1 | 1 | `battery_pct` | 0–100 |
| 2 | 1 | `wifi_connected` | 0/1 |
| 3 | 1 | `rssi` | `i8` bit pattern |
| 4 | 4 | `fetch_age_secs` | LE `u32`; `u32::MAX` = "never fetched" |
| 8 | 2 | `station_input` | LE `u16`, `StationIdx` ordinal (`Unknown`=65534, `None`=65535) |
| 10 | 1 | `station_input_row` | `0xFF` = idle |
| 11 | 1 | `station_input_col` | `0xFF` = idle |
| 12 | 1 | `board_id` | 0–3 |
| 13 | 1 | `tx_power` | `i8`, quarter-dBm (divide by 4 for dBm) |
| 14 | 36 | `led_rgb` | 12 × (r, g, b), in `Led` ordinal order |
| 50 | 1 | checksum | XOR of bytes 0–49 |

Built by `build_frame` in `serial_status.rs`. This is a recent format (git history: `3733354 serial: expand frame to 51 bytes, add tx_power, skip when USB idle`) — if you're looking at an older capture or an out-of-date web client, check the frame size matches before assuming a parsing bug.

## Version frame — 34 bytes, magic `0xAC`

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | magic (`0xAC`) |
| 1 | 32 | version string, space/zero-padded to `VERSION_STR_LEN=32` |
| 33 | 1 | checksum |

Carries `GIT_VERSION` (see [[Build-and-CI#Git version embedding]]). Sent once at startup, then every ~10s so late-connecting clients still learn the firmware version.

## Consuming this protocol

If you're changing this format, both sides need to move together:
- Firmware: `build_frame` / `build_version_frame` in `src/tasks/serial_status.rs`.
- Web: `web/src/serial-frame.ts` (parsing) and whatever in `web/src/main.ts`/`status.ts` renders the parsed fields.

There's no version negotiation — a size or field-order mismatch will just desync until the checksum-based resync kicks in and (at best) drops frames, or (at worst) misparses a same-length frame with shifted fields. Keep `FRAME_SIZE`/`VERSION_FRAME_SIZE` and the TS parser's expected sizes in lockstep by hand.
