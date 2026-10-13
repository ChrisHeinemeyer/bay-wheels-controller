---
tags: [task, wifi]
---

# `wifi_connect.rs` (`src/tasks/wifi_connect.rs`)

**Rewritten (2026-08-04, `light-sleep-wifi-cycle` branch).** This is no longer a
spawned task — there is no `wifi_connect_task` anymore. It's now a pair of plain
async helper functions called by [[Task-Fetch]], which owns the `WifiController`
for its entire lifetime and drives every connect/reconnect itself. See
[[Power-Management]] for why: the radio is now fully torn down between fetch
cycles rather than staying associated with modem-sleep, so there's no longer a
"connection persists indefinitely" state for a separate task to babysit.

## `setup()` — runs once, at boot

Called once by `fetch_task` before its main loop, before the WiFi teardown/reconnect
cycle starts:
1. Set STA mode, start the controller, apply max TX power.
2. Active-scan all channels (100ms min/max dwell) to find the target SSID's AP
   details (channel, auth method, RSSI) — returned as an `ApInfo`.
3. Build a `ClientConfig` from the scanned AP info, with `listen_interval(10)`, and
   `set_config()` it onto the controller.

This scan/config only happens **once** — every later reconnect via `connect()`
reuses it (assumes `set_config`'s state survives `stop_async()`/`start_async()`;
flagged for hardware verification, see the plan this branch implements).

## `connect()` — runs every cycle

Takes the caller's remembered `tx_power` (see below), applies it, calls
`controller.connect()`, then polls `is_connected()` every 100ms up to a ~500s bound
(re-issuing `disconnect()`+`connect()` every 5s if stuck) — the same retry shape as
the old one-shot boot connect, just returning `Result<i8, WifiError>` instead of
panicking, since a failed reconnect is now a routine, retryable event rather than a
fatal boot error.

## Adaptive TX power — now owned by `fetch_task`, re-based on successful cycles

The old "step down every 3 minutes of stable connection" clock doesn't exist
anymore — each connection now only lives a few seconds. `fetch_task` carries
`tx_power`/`success_streak` as loop-local state (no signal or static needed, since
only `fetch_task` ever touches the controller) and calls `connect(controller,
tx_power)` each cycle:
- On a successful connect+fetch: step `tx_power` down by `TX_POWER_STEP` once
  `SUCCESSES_BEFORE_STEP` consecutive successes accumulate (clamped at
  `TX_POWER_MIN`).
- On any failure: reset to `TX_POWER_MAX` and zero the streak — same "lock at max
  after trouble" intent as the old `tx_locked` flag, just triggered per-cycle.

`apply_tx_power()` (the raw `esp_wifi_set_max_tx_power` FFI wrapper) is unchanged.

## Relationship to `fetch_task`

`fetch_task` now owns the `WifiController` value outright (moved into it from
`main.rs` at spawn time) and drives the whole connect → fetch → teardown →
sleep cycle itself, including `set_power_saving()` around the handshake window.
See [[Task-Fetch]] for the full cycle and [[Power-Management]] for why full
teardown replaced modem-sleep-and-stay-connected.
