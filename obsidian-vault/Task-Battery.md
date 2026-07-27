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
`DISCHARGE_CURVE_MV`, a 15-point (pack mV, percent) lookup table, highest voltage first, with
linear interpolation between the two bracketing breakpoints (clamped to 100%/0% outside the
table's range). This replaced an earlier straight-line map between a depleted and fresh
voltage, which overstated capacity through alkaline's long flat plateau and understated how
little was left once voltage started sliding near depletion.

The table is derived from a commonly-cited single-cell alkaline discharge curve
(1.50V/1.40V/1.30V/1.20V/1.10V/1.00V/0.90V at 100/83/62/42/23/8/0%), scaled ×3 for the series
pack and interpolated to finer steps — still an approximation (real discharge shape depends on
drain current, temperature, and cell-to-cell balance), but it should track actual state of
charge more closely than the old linear map, especially through the mid-range plateau.

See [[Power-Management]] for whether 5s polling resolution is worth the periodic wakeup it costs.
