# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.1] - 2026-07-27

### Changed

- AL5887 LED driver chip is now disabled (`set_chip_enable(false)`) when the selection goes
  idle, instead of just zeroing all 12 LEDs' registers — cuts the driver's own quiescent draw,
  not just LED current, and re-enables it only when coming back out of idle.
- `battery_task` now samples every 30 seconds instead of every 5, removing a periodic wakeup
  for negligible loss of freshness.

### Fixed

- LEDs left on from a previously selected station could stay lit after selecting a new one.
  `Al5887::set_vec_led` only writes registers for the LEDs it's given and never clears ones
  that aren't in the new list, so switching to a station with fewer lit LEDs than the last one
  left the extras stuck on. All 12 LEDs are now explicitly zeroed on touch-release (going
  idle), so they're already off by the time the next selection lights new ones.

## [1.2.0] - 2026-07-26

### Added

- Adaptive WiFi TX power stepping: starts at 15 dBm, steps down 0.5 dB every 3 minutes of a
  stable connection to a 10 dBm floor, and re-maxes after any disconnect; live RSSI reported
  over the serial status frame.
- WiFi modem power-save (`PowerSaveMode::Maximum`, `listen_interval=10`), later widened to
  cover the entire life of a reused HTTPS connection rather than just the idle gap between
  fetches.
- Persistent HTTPS connection reuse across GBFS poll cycles instead of a fresh DNS+TCP+TLS
  handshake every 60 seconds, falling back to a full reconnect only when the connection is
  actually lost.
- Adaptive touch-grid poll rate: 500 ms while idle, 150 ms for 10 seconds after a touch.
- 15-point lookup-table battery percentage estimator for 3×AA alkaline packs, modeling the
  real (flat-then-cliff) discharge curve instead of a straight-line approximation between a
  depleted and fresh voltage.
- Serial status frame expanded to 51 bytes and now includes `tx_power`; sending is skipped
  when no USB host is reading it.
- WiFi-status LED now breathes during first connect instead of a static pattern.
- Obsidian vault documenting the firmware architecture, auto-loaded into project config.
- Rust build cache in CI.

### Changed

- Removed the WiFi sniffer/promiscuous mode and trimmed smoltcp features — it was blocking
  modem sleep.
- Reduced various task polling rates to improve battery life.

### Removed

- Boot-time LED0 flash and a per-cycle fetch-age debug log line.

## [1.1.0] - 2026-04-11

### Added

- Battery voltage sensing (`battery_task`): ADC read plus a linear voltage-to-percentage
  estimate.
- "Fetch age" LED indicator showing how stale the displayed station data is.

### Fixed

- Station ID mapping for new control-board revisions.
- GitHub Pages deployment and a CORS/bundling issue in the companion web app.

### Removed

- Leftover boot-time "blinky" LED test code.

## [1.0.0] - 2026-03-11

First stable release.

### Added

- Support for a new physical control-board revision.
- Explicit `(row, column)` station-grid mapping.

### Changed

- Increased GBFS poll frequency.

### Fixed

- Station ID lookup.
- LED display issues.

## [0.1.7] - 2026-03-10

### Changed

- Minor project description update.

## [0.1.6] - 2026-03-10

### Added

- Git version info embedded in firmware.

### Changed

- Improved per-board station mapping.
- Hardware pin addressing fix.

## [0.1.5] - 2026-03-09

### Added

- Web UI: view which stations are currently being pressed on the touch grid.
- Status page in the companion web app.
- `use-env` Cargo feature for loading WiFi credentials from `.env` at build time.
- Input-map generator tool to help build the per-board touch-grid mapping.

## [0.1.4] - 2026-03-09

### Added

- Write WiFi credentials to flash after provisioning; working companion web UI.

### Fixed

- Misc fixes.

## [0.1.3] - 2026-03-08

### Fixed

- Serial port handling issue.

## [0.1.2] - 2026-03-08

### Fixed

- GitHub Actions workflow fixes.

## [0.1.1] - 2026-03-08

### Added

- Web-based firmware updater.

### Fixed

- Build fixes.

## [0.1.0] - 2026-03-08

Initial firmware bring-up.

### Added

- GBFS station-status fetching and JSON parsing.
- AL5887 LED driver bring-up (shift-register input, PWM-driven station display).
- WiFi credential storage in NVS flash.
- `.env`-based configuration for local development.
- GitHub Actions CI workflow.

### Changed

- Switched target chip from ESP32-C6 to ESP32-S3.
- Replaced heap-allocated `alloc::vec::Vec` with `heapless::Vec`.

[Unreleased]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v1.2.1...HEAD
[1.2.1]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.7...v1.0.0
[0.1.7]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases/tag/v0.1.0
