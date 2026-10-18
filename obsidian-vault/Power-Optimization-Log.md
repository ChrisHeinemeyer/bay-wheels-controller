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
