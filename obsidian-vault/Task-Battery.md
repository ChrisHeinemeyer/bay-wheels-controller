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
// 3× AA alkaline: ~3000mV depleted → ~4500mV fresh
let pct = ((battery_mv - 3000.0) / (4500.0 - 3000.0) * 100.0).clamp(0.0, 100.0);
```

The 3000–4500mV range is a linear approximation of alkaline AA discharge, which is not actually linear (alkalines hold voltage relatively flat for most of their life, then fall off a cliff near the end) — so this percentage should be read as "rough state of charge," most accurate near the low end, not as a precise fuel gauge. Worth knowing if the reported battery % seems to sit near 100 for a long time and then drop quickly.

See [[Power-Management]] for whether 5s polling resolution is worth the periodic wakeup it costs.
