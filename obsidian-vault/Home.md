---
tags: [index]
---

# Bay Wheels Controller — Vault

An Obsidian vault documenting the `bay-wheels-controller` firmware: an ESP32-S3 device that displays live Bay Wheels (Lyft) bike-share availability on a physical LED board, wired to a touch grid so a user can pick which station to view.

Open this folder as an Obsidian vault (`Open folder as vault`) to get backlinks and the graph view. It's plain markdown, not checked into git yet — see [[Build-and-CI#Repo layout]] for where it sits.

## Start here

- [[Architecture]] — the big picture: hardware, boot sequence, task graph, how data flows from GBFS to an LED.
- [[Power-Management]] — battery-life posture: what's already optimized (adaptive TX power, modem sleep, adaptive poll rate) and where the remaining budget is going.
- [[Open-Questions]] — quirks and rough edges I noticed while reading, not necessarily bugs.

## By subsystem

- [[Shared-State]] — `signals.rs`: the `Signal`s and `STATUS` mutex that glue tasks together.
- [[Task-Wifi-Connect]], [[Task-Fetch]], [[Task-Station-Parser]], [[Task-Station-LEDs]], [[Task-Input-Read]], [[Task-Battery]], [[Task-Serial-Status]] — one note per Embassy task.
- [[Hardware-Drivers]] — the AL5887 LED driver and shift-register input grid (both bit-banged SPI), plus the battery ADC divider and board-ID GPIO straps.
- [[Data-Model]] — `stations.rs` / `grid.rs`: the `StationIdx` enum, per-board `(row, col) → station` map, and the GBFS UUID lookup table.
- [[Networking]] — `network.rs`, embassy-net stack setup, DHCP.
- [[Serial-Protocol]] — the binary status frame sent over USB-Serial-JTAG to the companion web app.
- [[Provisioning-and-Config]] — first-boot WiFi setup over serial, NVS credential storage.
- [[Companion-Tools]] — the two Vite/TS web apps (`web/`, `tools/`) that flash firmware, monitor status, and generate the board-mapping data this firmware consumes.
- [[Build-and-CI]] — Cargo features/profiles, `build.rs` codegen, git hooks, CI.

## One-paragraph mental model

At boot, the firmware reads a GPIO strap to find out which physical board it's soldered onto (`BoardId`), connects to WiFi using credentials from flash (or drops into a serial provisioning wizard if none exist), then runs six-ish independent Embassy tasks forever: one polls the Bay Wheels GBFS feed every 60s and streams-parses out ~120 stations of interest, one polls a touch grid to see which station the user is pressing, one drives a 12-LED-per-station RGB driver chip based on the currently-selected station's live bike counts, one samples battery voltage, one manages WiFi TX power/reconnects, and one reports a binary status frame over USB serial for the companion web UI to display. Tasks never call each other directly — they communicate through a handful of `embassy_sync` `Signal`s and a shared `STATUS` mutex (see [[Shared-State]]).
