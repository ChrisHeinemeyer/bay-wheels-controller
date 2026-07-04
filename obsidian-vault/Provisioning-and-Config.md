---
tags: [provisioning, config]
---

# Provisioning and Config

## Entry points (`src/bin/main.rs`)

Two ways into `provisioning::run_provisioning` (blocking, `-> !`, runs *before* Embassy starts):
1. GPIO2 held low at boot (the provisioning button).
2. No WiFi credentials found in NVS (e.g. first boot after a fresh flash).

Both paths grab a fresh `UsbSerialJtag` in **blocking** mode — distinct from [[Task-Serial-Status]]'s async instance used during normal operation. Since provisioning runs before any tasks are spawned, there's no ownership conflict, just sequencing: whichever path is taken, `FLASH` and `USB_DEVICE` peripherals must not already be borrowed elsewhere at that point in `main`.

## The provisioning wizard (`src/provisioning.rs`)

A blocking, hand-rolled line-editor over serial (backspace handling, CRLF normalization, character echo) that:
1. Prompts for SSID, **re-printing the prompt every 2 seconds** (`read_line_with_beacon`) while waiting for input — this exists specifically so a browser-based serial client (the [[Companion-Tools|web app]]) that connects *after* the prompt was first written still sees it, rather than staring at a blank terminal.
2. Prompts for password with masked echo (`*` per character).
3. Shows a confirmation summary (password shown as up to 8 `*`s regardless of actual length) and asks y/n.
4. On yes: steals a fresh `FlashStorage` (see the `unsafe`/safety comment at `provisioning.rs:83` — sound because the original `_flash` passed in is still alive and never itself used to write, so there's no real aliasing) and calls `wifi_config::save_credentials`.
5. On NVS write failure: reboots *without* looping to retry in-place. The comment explains why — NVS writes are known to fail on the very first boot after a fresh flash/power-on, and a full reboot fixes that flash state; provisioning resumes automatically on the next boot since no credentials were saved.
6. On success or explicit cancel: `esp_hal::system::software_reset()`.

## Credential storage (`src/wifi_config.rs`)

- Uses `esp-nvs` directly against a fixed partition window: offset `0x9000`, size `0x6000` (24 KiB) — the comment notes this matches the *default* ESP-IDF NVS partition location, so it assumes the flashed partition table hasn't been customized. If you ever change the partition table (`partitions.csv` or equivalent), this offset/size needs to move with it — nothing here reads the partition table to find NVS dynamically.
- Namespace `"wifi"`, keys `"ssid"`/`"password"`, both stored as plain strings — **not encrypted**. Anyone with physical flash access (or a copy of a flash dump) can read the WiFi password in plaintext.

## Two credential paths at compile time

`main.rs` branches on the `use-env` Cargo feature (see [[Build-and-CI#Cargo features]]):
- **`use-env` on**: credentials come from `env!("SSID")`/`env!("PASSWORD")`, baked in at *compile* time from a `.env` file (via `build.rs`). Convenient for development, but means the binary itself embeds the WiFi password — don't publish a `use-env` build.
- **`use-env` off** (the normal/shipped path): credentials come from NVS at runtime, as described above.
