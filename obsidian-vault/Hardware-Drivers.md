---
tags: [hardware, spi]
---

# Hardware Drivers

Two custom SPI devices, both **blocking** SPI (`esp_hal::spi::master::Spi<'d, Blocking>`) wrapped in an async-looking API (the `async fn`s don't actually yield mid-transfer — they're blocking calls inside an `async fn` signature, which is fine here since each transfer is short and at 100kHz, but it does mean the executor is stalled for the duration, not truly cooperative). Both run at the same 100 kHz SPI clock, on separate buses (SPI2 for input, SPI3 for LEDs) so they don't contend.

## AL5887 LED driver (`src/spi_devices/al5887/`)

- **Bus**: SPI3, `Mode::_0`, 100 kHz, manual CS (inverted polarity — hence "manual" rather than hardware CS).
- **Pins** (from `main.rs`): CS=GPIO10, SCK=GPIO12, MISO=GPIO13, MOSI=GPIO11, `led_en`=GPIO8, `led_rst_n`=GPIO9.
- **Protocol** (`al5887.rs`, `SpiFrame`): each register access is 2 bytes — `[(register << 1) | write_bit, value]`. A read does a write of the address byte followed by a separate read of the response byte.
- **Registers** (`registers.rs`): `DeviceConfig0/1`, `LedConfig0/1`, per-bank and per-LED brightness/color registers (`Rgb{0..11}Brightness`, `R/G/B{0..11}Color`), plus `Reset`, `Flag`, `LedGlobalDimming`, `FaultWait`, `MaskAndClr`. Only chip-enable (`DeviceConfig0` bit 6) and the per-LED brightness/color registers are actually used by this firmware — `LedGlobalDimming`, `FaultWait`, `MaskAndClr`, and the bank registers are defined but unused.
- **Init sequence** (`init_driver`): `led_en` high, pulse `led_rst_n` low→high (1ms low), then write chip-enable=true. Called once at [[Task-Station-LEDs]] startup and never repeated — there's no runtime recovery path if the chip glitches (e.g. from an ESD event or brownout) other than a full device reboot.
- **12 addressable LEDs** (`enums::Led`, `Led0..Led11`), each independently settable to an 8-bit brightness + RGB color via `set_led_brightness_color` (4 register writes per LED: brightness, R, G, B).
- **Power**: chip-enable is set once and never turned back off, even when the display is fully blanked (`StationIdx::None` sets all colors to 0 but leaves the chip enabled) — see [[Power-Management]].

## Shift-register input grid (`src/spi_devices/shift_register.rs`)

- **Bus**: SPI2, `Mode::_2`, 100 kHz, manual CS (active-high).
- **Pins** (from `main.rs`): CS=GPIO33, SCK=GPIO35, MISO=GPIO36. No MOSI — this is a read-only shift-in device (parallel-in/serial-out register(s) behind the touch grid), the mirror image of the AL5887's write-mostly SPI-out.
- **Read cycle**: raise CS, clock in 5 bytes (40 bits), lower CS. The 40 bits are split: bits 39–22 (18 bits, MSB-first) = column scan, bits 19–0 (20 bits, MSB-first) = row scan. Each half is scanned for the *first* `0` bit (active-low — an untouched line reads high); that bit index is the active row/column. All-high in a half means "idle" (`0xFF` sentinel).
- **Known hardware quirk** (comment at `shift_register.rs:65`): "Temporary issue because not all lines are pulled up" — any row/column reading `>= 6` is forced to `IDLE` even though up to 18/20 bits are theoretically decodable. This caps the addressable grid at 6×6=36 cells today, likely a stopgap for boards where only the first 6 lines of each shift register bank are actually pulled up correctly. Worth checking hardware revision before assuming a >6 row/col design "just works" — this code currently prevents it.
- Only two bits of data actually determine a single (row, col) reading — the design assumes exactly one cell is active at a time (a single touch), with no explicit multi-touch handling; a genuine multi-touch event would just resolve to whichever bit is lowest-indexed in each half.

## Battery ADC (`src/bin/main.rs` + `src/tasks/battery.rs`)

- GPIO5 → ADC1, `Attenuation::_2p5dB`, hardware-calibrated (`AdcCalLine`).
- Sits behind a 22kΩ/100kΩ resistive divider (see [[Task-Battery]] for the voltage math) off the 3×AA battery pack.

## Board-ID straps

- GPIO37 (bit 0) and GPIO38 (bit 1), both internally pulled up, read once at boot then dropped (freeing the GPIOs). A populated resistor to GND on either pin clears that bit. Unpopulated boards read `0b11` = `BoardId::Board3`. See [[Data-Model#BoardId and BOARD_STATION_MAP]] for what this selects.
