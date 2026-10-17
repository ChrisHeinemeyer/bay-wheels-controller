---
tags: [task, power]
---

# `battery_task` (`src/tasks/battery.rs`)

Simplest task in the codebase. Every 5 seconds:

1. Blocking one-shot ADC read (`nb::block!(adc1.read_oneshot(&mut pin))`) on GPIO5, attenuation `_2p5dB`.
2. Converts millivolts → percentage in `convert_to_percentage`.
3. Writes `STATUS.battery_pct`.

## Voltage math

```rust
let voltage_div_ratio = 22.0 / (100.0 + 22.0);   // 22kΩ low-side, 100kΩ high-side divider
let battery_mv = adc_mv / voltage_div_ratio;      // undo the divider
```

`battery_mv` (pack voltage, 3 cells in series) is then mapped to a percentage via
`DISCHARGE_CURVE_MV`, a 16-point (pack mV, percent) lookup table, highest voltage first, with
linear interpolation between the two bracketing breakpoints (clamped to 100%/0% outside the
table's range).

### Calibration (2026-10-17)

The bottom half of the table is measured from log-server telemetry. Board3 on v1.3.1 ran
continuously from 2026-10-13 11:31 to its last report at 2026-10-17 04:08 (88.6h, 2,515
reports). The duty cycle was constant, so elapsed time is proportional to charge drawn.
Voltages are a rolling 1h 75th percentile, which rejects the ~17% of samples that land on a
radio TX burst and read ~130 mV (pack) low.

| Charge used in segment | Pack mV | mV/cell |
|---|---|---|
| 0% | 3954 | 1.318 |
| 20% | 3887 | 1.296 |
| 40% | 3821 | 1.274 |
| 50% | 3760 | 1.253 |
| 60% | 3732 | 1.244 |
| 70% | 3660 | 1.220 |
| 80% | 3566 | 1.189 |
| 90% | 3488 | 1.163 |
| 95% | 3405 | 1.135 |
| 100% (last report) | 3327 | 1.109 |

- **0% = 3330 mV pack (1.11V/cell)**, which is where this board stopped reporting. The old
  table put 0% at 2700 mV and showed **25%** at the moment the board died.
- The cells were **not fresh** at the start of the log (1.359V/cell on 2026-10-12, with a
  history of PPK2 back-feeding). So the segment start (1.318V/cell) is not 100%. It is
  placed at **55%**, an estimate from the generic alkaline shape.
- Above 3955 mV the table keeps the generic single-cell alkaline shape (×3), rescaled to
  55-100%. This part is unverified. A run from fresh cells to cutoff would fix the 55% split
  and the top of the curve.

See [[Power-Management]] for whether 5s polling resolution is worth the periodic wakeup it costs.
