---
tags: [power, measurements]
---

# Power Optimization Log (2026-10-17)

One day of PPK2-driven experiments on the `power-optimization` branch, each
"round" a firmware build measured on the bench. Context and the overall budget
live in [[Power-Management]]; this note is the record of what was tried, what
it did, and what stayed.

## Setup

- **PPK2** in source-meter mode at 4.5V in place of the AA pack, 100kS/s
  averaged into 1ms bins. `tools/ppk2/ppk2.py` (uv script):
  `on`/`off`/`cycle`, `measure -s N --csv --json --progress`, and `windows`
  to split a CSV into fixed windows. Time is classed by 1ms-bin current:
  **asleep** < 25mA, **awake** (CPU on, radio off) 25-80mA, **radio** ≥ 80mA.
- **Telemetry** from each cycle to the Home Assistant add-on
  ([[Telemetry-and-Recovery]]): `connect_ms`, `dhcp_ms`, `fetch_ms`,
  `prev_radio_on_ms`, plus fetch sub-phases (`handshake_ms`, `ttfb_ms`,
  `body_ms`, `parse_ms`, `body_bytes`), `src` (mirror/lyft) and `lease`
  (reused/dhcp).
- **Flashing:** USB VBUS powers the board, so each flash is: PPK2 off, plug
  USB, flash within the 20s no-sleep boot window, unplug, PPK2 on. Once the
  board light-sleeps, its USB-JTAG enumerates but doesn't answer.
- Build-time knobs (`option_env!`, read by `power::parse_u32`): `SLEEP_CFG`,
  `MIN_NAP_US`, `FETCH_GAP_S`, `DHCP_DISCOVER_MS`, `DHCP_REUSE`, `GARP`,
  `TX_POWER_FIXED`, sweeps `SLEEP_SWEEP_S`/`NAP_SWEEP_S`.

## Results by round

| Round | Change | Result |
|---|---|---|
| 1 | Light-sleep `RtcSleepConfig` levels 0-3 (`SLEEP_SWEEP_S`) | Asleep current: L0 11.9mA, L1 11.7, **L2 4.4**, L3 4.3. Kept **level 2** (ESP-IDF light-sleep regulator values, 8M + XTAL off). |
| 2 | DHCP DISCOVER retry 250ms | Stock smoltcp changes the xid on every retransmit, so a late OFFER was dropped and DHCP took 6.2s. **Patched smoltcp** to keep the xid. |
| 3 | `MIN_NAP_US` sweep 5000/4000/3000/2500 | No effect. Budget at that point: **22.5mA** = asleep 79.5% × 4.5mA + awake 13.9% × 46mA (6.5mA) + radio 6.6% × 189mA (12.4mA). Radio window: connect 0.1s, DHCP 1.25-2s, Lyft fetch 4.1s. |
| 4 | **GBFS mirror** on the HA add-on (`gbfs-mirror` feature): plain HTTP, only this board's stations and the 3 parsed fields | Fetch 4.1s → **~50ms**, body 270KB → 3.9KB. Average ~13.6mA. |
| 5 | **Reuse the DHCP lease** as a static config until 90% of T1 (router gives 48h) | `dhcp_ms` 0, but the mirror TCP handshake took ~2,011ms: a dead period after association. |
| 6 | smoltcp ARP: 1h cache, 200ms request gap | No change (later found the cache was flushed every cycle anyway, see round 9). |
| 7 | Mirror TCP connect retried every 250ms | Handshake 2.0s → 1.26-1.52s. **10.4mA**. |
| 8 | Count received frames (broadcast vs unicast) | Broadcasts arrive 276-363ms after association, unicast only at ~1.0-1.5s. |
| 9 | Numbered UDP probes every 20ms to the add-on, echoed back | 9.8mA; most cycles clean. On a bad cycle the host got exactly probes 0-8 in one 13-19ms burst: **the socket was waiting for ARP**. `update_ip_addrs` flushes the ARP cache on every `set_config_v4`, and a socket re-asks ARP only every 1s. |
| 10 | **smoltcp: flush only when the addresses change; per-socket ARP retry 200ms** | Uplink now works from the first probe. But **unicast toward the board** is dropped (not delayed) for ~1.2-1.3s: the first echo the board gets is #56-67. |
| 11 | A-MPDU RX off (15s gap) | Every cycle ~2.76s (15/15). No help. |
| 12 | **Gratuitous ARP** right after connect (15s gap) | First 6 cycles bad (2.76s), then clean from 110s on: handshake 9-18ms, radio-on ~175-255ms. |
| 14 | Round 12 at the real 120s gap, 48min | 6 clean, 17 bad. **12.2mA overall: 9.6mA in the clean phase, ~11.9mA in the bad one.** |
| 13 | 100ms wait between disconnect and radio stop | No change (2.76s at 15s gap). |
| 15 | No disconnect (deauth) before radio stop | No change. |
| 16 | Doze-and-probe: modem sleep during the dead period, probe every 100ms | Dead period ~205mA instead of ~230. Power save barely engages. |
| 17 | Same, probe every 500ms | ~167mA average during the dead period, plus up to 500ms latency. Not worth it. |

## The post-association unicast loss (unsolved)

After every fast re-association (~107ms connect), unicast frames **toward**
the board are lost for 1-3s while the uplink and received broadcasts already
work. What's known:

- It comes in phases: runs of clean cycles, then runs of bad ones. It is
  worse when the board was off the air for less time (15s gap: ~2.7s every
  cycle; 120s gap: ~1.1-2.3s, or none; boot after minutes unpowered: none).
- Not changed by: ARP (cache, retry timing, gratuitous ARP alone), A-MPDU RX,
  deauth timing, skipping the deauth.
- So it's in the AP or in the closed ESP WiFi blob. Untested: the router's
  model/WiFi security mode, WPA3/SAE, PMKSA caching and 802.11r (`ft_enabled`)
  in the station config.
- Cost: ~2.3mA of average current in a bad phase at the 120s gap.

## What stayed

- `SLEEP_CFG=2` and `DHCP_DISCOVER_MS=250` are now the defaults.
- `gbfs-mirror` feature + add-on mirror (falls back to Lyft on any failure).
- DHCP lease reuse (`DHCP_REUSE`, default on).
- Patched smoltcp (`patches/smoltcp-0.12.0`, all changes marked `PATCHED`):
  DHCP xid kept across DISCOVER retries; lease/T1 exposed; ARP cache 1h and
  kept across same-address reconfigs; 200ms ARP retries (cache and
  per-socket); `ANNOUNCE_ARP` for the gratuitous ARP.
- Mirror connect retry every 250ms; gratuitous ARP after connect (`GARP`).
- Removed after testing: frame counters, UDP probe/echo (firmware and add-on),
  A-MPDU/teardown/deauth knobs, doze-and-probe.

## Where the current goes now (round 14, 120s gap)

| Part | Share × current | Average |
|---|---|---|
| Asleep | 87.4% × 4.33mA | 3.8mA |
| Awake, radio off | 10.9% × 46.5mA | **5.1mA** |
| Radio on | 1.8% × 191mA | 0.3mA (clean) to 2.4mA (bad phase) |
| **Total** | | **9.6-12mA** (was ~20mA on v1.3.1) |

**Next:** awake time is the largest part. ~11% of the time the CPU is up at
~46mA with the radio off, mostly short wakes between naps.

## Awake time (rounds 18-20)

From round 14's 1ms bins, between fetches the CPU wakes **16.4/s**: ~12/s
wakes of 4-5ms and ~3.3/s of 13-15ms. Every wake costs at least ~4ms
because a nap ends `WAKE_MARGIN_US` (2ms) before the alarm and the CPU then
waits awake for it (`near_deadline_waits`).

Telemetry (`sleep-trace`) now reports what wakes the CPU while asleep:
`timer_wakes_total`, `armed_total` (armed timer lengths), and with
`sleep-trace` the per-task embassy polls, radio timer callbacks by address
(`radio_timers`) and timed sleeps per esp-rtos thread (`thread_sleeps`). Task
order: 0 main, 1 net, 2 battery, 3 LEDs, 4 fetch, 5 input, 6 serial.

| Round | Change | Result |
|---|---|---|
| 18 | Wake-source telemetry | Steady state ~18 nap wakes/s, ~7/s from embassy tasks: LEDs ~4.4/s, input ~2.2/s, serial ~0.5/s. The input task signalled the LED task every 500ms poll even with no change, and each signal cost two LED wakes. |
| 19 | **LED fix** (signal only on change) + **WiFi de-init between cycles** (drop the `WifiController`, re-create it with `esp_radio::wifi::new` from the leaked radio init) | Crash-looped: embassy-net answered a frame left in the RX queue after the drop, and `esp_wifi_internal_tx` jumped into the de-initialised driver (LoadProhibited, PC 0x10). |
| 19b | Same, clearing the IPv4 config and draining RX for 50ms before the drop | Stable, heap steady at 56.8KB after teardown. LED task no longer polled while asleep; embassy wakes ~2.7/s. **Zero radio timer callbacks while asleep**, yet still **~14 nap wakes/s**. Every cycle cold-starts the radio: connect ~1.9s (was ~0.1s), radio on ~4s per cycle. **13.9mA, worse** than round 14 (9.6-11.9mA). De-init removed. |
| 20 | Count timed sleeps per esp-rtos thread (entry function address) | Running. |

**Lessons:**
- The leftover ~11 wakes/s are not the WiFi blob's ets timers; something
  else sets the alarm. Round 20 counts which esp-rtos thread sleeps with a
  timeout.
- Re-initialising the radio costs ~1.8s of extra radio time per cycle (PHY
  calibration and full authentication). It could only pay off at much longer
  fetch gaps.
- Before ever dropping the WiFi driver again, stop embassy-net from
  transmitting into it (clear the IP config and drain RX), or it crashes.
