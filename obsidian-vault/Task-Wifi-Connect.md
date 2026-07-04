---
tags: [task, wifi]
---

# `wifi_connect_task` (`src/tasks/wifi_connect.rs`)

Owns the `WifiController` for the entire program lifetime. Two phases:

## Phase 1 — connect (runs once)

1. Set STA mode, start the controller, apply max TX power (`TX_POWER_MAX = 84` quarter-dBm = 21 dBm).
2. Active-scan all channels (100ms min/max dwell) to find the target SSID's AP details (channel, auth method, RSSI).
3. Build a `ClientConfig` from the *scanned* AP info (not hardcoded), with `listen_interval(10)` — the radio will sleep for 10 beacon intervals (~1s) whenever modem power-save is later enabled. Power save is deliberately **not** enabled here; ownership of power-save mode is handed to [[Task-Fetch]] (see the comment at `wifi_connect.rs:153` — enabling it before DHCP/first DNS lookup was found to race and cause failures).
4. `controller.connect()`, then poll `is_connected()` every 100ms for up to 500s, re-issuing `disconnect()` + `connect()` every 5s if still not connected (some boards apparently get stuck and need this kick).

## Phase 2 — maintenance loop (runs forever, every 5s)

- If disconnected: reconnect, and if TX power had been stepped down, **lock it at max** for the rest of the session (`tx_locked = true`) — the assumption baked in here is that a disconnect while at reduced power means this particular board's TX circuitry can't sustain lower power, so don't risk it again.
- If connected: update live RSSI into `STATUS`, and if not locked and still above the floor, count stable ticks and step TX power down by 2 (quarter-dBm) every `STABLE_TICKS_PER_STEP = 36` ticks (~3 minutes) until `TX_POWER_MIN = 60` (15.0 dBm).

## Adaptive TX power state machine

```
TX_POWER_MAX (84, 21.0dBm) ──connect──▶ every 3 min stable ──▶ step -2 ──▶ ... ──▶ TX_POWER_MIN (60, 15.0dBm)
        ▲                                                                              │
        └──────────────────── disconnect (locks here if was stepping) ◀────────────────┘
```

This is a real power optimization already in place — see [[Power-Management#Already implemented]]. The "lock at max after a disconnect mid-step" behavior means a board that experiences one bad RF moment (e.g. someone walks in front of it) will run at full TX power for the rest of that boot, even after signal recovers — it never re-attempts stepping down after locking. Worth knowing if you ever see a board stuck at high TX power for no obvious reason: check its `wifi_connected` history in the serial frame, not just current RSSI.

## Relationship to `fetch_task`

This task and [[Task-Fetch]] both touch the radio's power-save mode indirectly: this task never calls `esp_wifi_set_ps` at all; `fetch_task` toggles it around each HTTP request. See [[Power-Management]] for the full picture of how these interact.
