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
- Build-time knobs (`option_env!`, read by `power::parse_u32`) were added
  for the experiments: `SLEEP_CFG`, `MIN_NAP_US`, `DHCP_DISCOVER_MS`,
  `DHCP_REUSE`, `GARP`, `TX_POWER_FIXED`, `AMPDU_RX`, `PMF`, sweeps
  `SLEEP_SWEEP_S`/`NAP_SWEEP_S`. v1.4.0 removed them all (the chosen values
  are constants now) except `FETCH_GAP_S` (seconds between fetches, default
  120; shorter cycles while testing).
- Ad-hoc analysis that isn't in the repo: per-task attribution (telemetry
  `task_polls_total` × PPK2 charge per wake) and the sniffer analysis below
  (`tshark` filters on the board's MAC).

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

## The post-association unicast loss: the Xfinity XB8 gateway

After every fast re-association (~107ms connect), unicast frames **toward**
the board are lost for 1-3s while the uplink and received broadcasts already
work. It comes in phases (runs of clean cycles, then runs of bad ones) and is
worse when the board was off the air for less time (15s gap: ~2.7s every
cycle; 120s gap: ~1.1-2.3s, or none; boot after minutes unpowered: none).
Cost: **~2mA** of average current in a bad phase at the 120s gap.

Firmware changes that didn't help: ARP (cache, retry timing, gratuitous
ARP), A-MPDU RX off (rounds 11, 28), a teardown wait (13), no deauth (15),
doze-and-probe (16-17), PMF off (27; had no effect at all: the ESP-IDF
driver ignores `pmf_cfg.capable` and always advertises PMF when the AP does).

**Sniffer captures (2026-10-18)** with macOS Wireless Diagnostics on the
2.4GHz channel (1, 20MHz) of the gateway, an **Xfinity XB8 (Technicolor,
v2.3)** running WPA2/WPA3 mixed mode, BSSID `72:13:01:06:21:cd`; board
`ac:27:6e:ae:e7:04`:

- With A-MPDU on: thousands of Block Ack Requests from the gateway (572 in
  one 3s gap, ~150 per 100ms in bursts). The board answers every one with a
  Block Ack, but the gateway keeps asking and sends almost no data.
- With A-MPDU off (round 28): the storms are gone but the gap isn't. The
  gateway's QoS Null frames to the board go out ~7 times each. Then, every
  ~50ms, the gateway sends the board an RTS, **the board answers CTS within
  microseconds** (seen by the sniffer), and the gateway sends another RTS
  instead of its data, 7 times, then gives up. The gateway ACKs the board's
  uplink throughout. After ~2.8s it starts accepting the board's replies.
- The Block Ack setup (ADDBA) is a protected action frame under PMF and is
  retried 5-7 times each way at the start of every connection.

**Conclusion:** the board answers correctly; **the gateway ignores its
ACK/CTS replies for 1-3s after each re-association**. Probably per-station
state the gateway keeps after a station leaves; clean phases are when it has
dropped that state. Not fixable in firmware. Fixes on the network side: a
separate plain (WiFi 4/5) access point for the board, or a router of one's own
with the XB8 in bridge mode. The XB8's admin page doesn't appear to offer an
802.11 mode or WPA2-only setting for 2.4GHz.

## What stayed (v1.4.0)

- Light-sleep config: ESP-IDF's light-sleep values (8M and XTAL off,
  digital rail dropped). Asleep 11.9 → 4.4mA.
- **TIMG1 divider fix** in the idle hook (round 26), and **no early-wake
  margin**: naps run to the deadline, tasks may run up to the wake-up
  latency late.
- Input task signals the LED task only on a change.
- `gbfs-mirror` feature + add-on mirror (falls back to Lyft on any failure).
- DHCP lease reuse until 90% of T1; DHCP DISCOVER retry 250ms.
- Mirror connect retry every 250ms.
- Patched smoltcp (`patches/smoltcp-0.12.0`, all changes marked `PATCHED`):
  DHCP xid kept across DISCOVER retries; lease/T1 exposed; ARP cache 1h and
  kept across same-address reconfigs; 200ms ARP retries (cache and
  per-socket).
- `sleep-trace` telemetry: per-task embassy polls while asleep.
- Removed after testing: frame counters, UDP probe/echo (firmware and add-on
  1.1.2), gratuitous ARP, WiFi de-init, A-MPDU/PMF/teardown/deauth knobs,
  doze-and-probe, esp-rtos timer/alarm counters, all sweep knobs.

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
| 20 | Count timed sleeps per esp-rtos thread (entry function address) | Only esp-rtos's radio `timer_task`, once per stretch. Not the wakers. |
| 21 | Count RTOS alarms by what they were armed for | All ~35/s for the embassy timer queue, yet only ~7/s embassy executor wakes. |
| 22 | Count early vs processed embassy alarms | **Every nap ended in one early alarm** (2,629 of 2,629); 698 real. 48min: **11.46mA**, awake 7.3% (LED fix alone: 5.1 → 3.4mA). |
| 23 | Keep TIMG1 in lockstep with SYSTIMER after each nap | No change. Neither counter moves by itself in sleep, so not drift. |
| 24 | Sample TIMG1's interrupt flag around each nap | Never raised there. |
| 25 | Histogram of how early | 5-50ms and 50ms+, far more than the 2ms wake margin. 48min: 11.44mA. |
| 26 | **Fix `timg1_t0_ticks_per_us`**: a prescaler DIVIDER of 1 divides by 2 (TRM, esp-hal), we used 1 | **Naps 2,664 → 697 per stretch, early alarms 0, wakes ~3/s of 2-3ms, awake 0.8%. ~6.7mA.** |

**The bug (round 26):** esp-rtos runs TIMG1 with DIVIDER = 1, which the
hardware treats as /2 (40 ticks/µs at 80MHz APB). The idle hook read it as
/1 (80 ticks/µs), so each nap covered only half the time to the alarm and
then pushed TIMG1 twice as far ahead. Every embassy timer took ~4 naps, each
ending in a spurious "early" alarm and a 2ms near-deadline wait.

## Where the current goes now (round 26, 120s gap)

| Part | Average | Notes |
|---|---|---|
| Sleep floor (board + chip) | **4.12mA** | ~62%. Board alone was ~1.8mA (ESP32 held in reset, v1.3.1). |
| Fetch cycle, radio on | ~2.1-3.2mA | 1.2-1.8s at ~220mA per cycle; **~70-80% of it is the AP's post-association unicast loss** (1.0-1.5s). |
| Input poll (500ms) | ~0.3mA | 2 wakes/s × ~140mA·ms |
| Serial status (2s) | ~0.07mA | 0.5 wakes/s × ~130mA·ms |
| Battery (30s), main, fetch CPU time | <0.05mA | |
| **Total** | **~6.7mA** | ~12 days on 3×AA (2,100mAh); v1.3.1 was ~20mA |

Per-task split: telemetry poll counts (`task_polls_total`) times the PPK2's
measured charge per wake; the bursts can't be told apart by period alone
because 500ms, 2s and 30s timers line up. Each wake was ~2.5ms at ~46mA,
mostly the idle hook's 2ms early-wake margin (removed in v1.4.0).

**Lessons from rounds 18-26:**
- The ~11 non-embassy wakes/s were not the WiFi blob's timers or threads:
  they were the idle hook's own TIMG1 tick-rate bug chopping each timer into
  ~4 naps.
- Re-initialising the radio costs ~1.8s of extra radio time per cycle (PHY
  calibration and full authentication). It could only pay off at much longer
  fetch gaps.
- Before ever dropping the WiFi driver again, stop embassy-net from
  transmitting into it (clear the IP config and drain RX), or it crashes.

## v1.4.0 (cleaned-up build, 120s gap)

Measured 2026-10-18 on the release candidate (`use-env,log-server,gbfs-mirror,sleep-trace`),
385s, all cycles in a clean gateway phase:

| Part | Average | Notes |
|---|---|---|
| Sleep floor (board + chip) | **4.16mA** | ~90% of the total now |
| Fetch cycle | 0.35mA | ~190-200ms of radio per cycle at ~230mA |
| Input poll (500ms) | 0.07mA | 2 wakes/s, ~1.2ms and ~34mA·ms each (was ~2.5ms, ~140mA·ms with the 2ms margin) |
| Serial status, battery, other | ~0 | they share wakes with the input poll |
| **Total** | **4.62mA** | |

Awake 0.2% of the time. In a bad XB8 phase the dead gap adds ~2mA, so the
real-world average is **~4.6-6.6mA**: about 13-19 days on 3×AA (2,100mAh),
against ~4.4 days on v1.3.1.

**Gateway refusing the board (2026-10-18 ~10:00):** after a morning of
hundreds of reconnects every ~18s, the XB8 refused to associate the board for
~20 minutes. Identical, previously working firmware also failed; it finally
connected after 145s of retries (`connect_ms` 145277). `wifi_connect::connect`
keeps the radio on for up to ~500s while retrying, so a refusal like that
costs ~200mA for its duration.

**Next options:** a separate access point for the board (removes the XB8's
~2mA dead gap); the sleep floor, see [[Sleep-Floor-Plan]].
