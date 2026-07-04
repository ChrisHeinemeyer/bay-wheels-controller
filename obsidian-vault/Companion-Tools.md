---
tags: [tooling, web]
---

# Companion Web Apps

Two separate Vite/TypeScript apps live alongside the firmware, both browser-based using WebSerial/WebUSB (Chrome/Edge only — no Safari/Firefox, per the README).

## `web/` — flasher + live status monitor

Dependencies: `esptool-js` (flashing over WebSerial), `leaflet` (map display).

Files (`web/src/`):
- `flash.ts` — drives `esptool-js` to flash a `.bin` (pre-built release or user-uploaded) over WebSerial. This is the "no command line" onboarding path described in the README.
- `firmware-version.ts` — presumably surfaces the version frame (see [[Serial-Protocol]]) or fetches release metadata for the "latest release" flashing option.
- `serial-frame.ts` — parses the binary status/version frames the firmware emits (see [[Serial-Protocol]] — verified byte-for-byte against `src/tasks/serial_status.rs`).
- `status.ts` / `main.ts` — presumably render the parsed status (battery, wifi, live LED mirror, station selection) as a live dashboard, likely with `leaflet` used to show the selected station on a map.
- `wifi.ts` — likely drives the provisioning wizard from the browser side (sending SSID/password over serial to `provisioning::run_provisioning`'s prompts, see [[Provisioning-and-Config]]).
- `generated/station-ids.ts` — **not hand-written**, produced by `build.rs` from `src/stations.rs` (see [[Data-Model#Generated artifact]]). Don't edit this file directly; edit `TARGET_STATIONS` in the Rust source and rebuild.

Hosted as a static site via GitHub Pages (README links to `https://chrisheinemeyer.github.io/bay-wheels-controller/`) as well as runnable locally (`npm run dev`).

## `tools/` — board-mapping authoring tool

Separate app, `"bay-wheels-board-mapper"`, dependencies `js-yaml` + `leaflet`. This is almost certainly how [[Data-Model#BoardId and BOARD_STATION_MAP]] gets authored in practice: `src/serial.ts` reads live shift-register (row, col) events from a connected board over WebSerial while `src/map.ts` (leaflet) lets a human click the corresponding real-world station on a map, and `src/yaml-io.ts` persists the resulting (row, col) → station-UUID associations to/from YAML.

`scripts/board-mapping-to-station-map.js` (run via `npm run board-map`) is the bridge back into Rust: it almost certainly reads that YAML and generates (or helps you hand-transcribe) the `BOARD_STATION_MAP` entries in `src/stations.rs` — i.e. this is the *reverse* direction of the `build.rs` codegen in `web/` (which goes Rust → TS; this tool's output goes YAML → Rust, likely manually pasted in rather than auto-injected, since `stations.rs` isn't listed as a `build.rs` output target for this data).

**Practical implication**: adding a new physical board layout (a new `BoardId` variant's entries in `BOARD_STATION_MAP`) is a two-step, two-repo-area workflow — map cells to stations using `tools/`, then hand-integrate the result into `src/stations.rs`. There's no single source of truth file that both the firmware and this tool read directly; the YAML is an intermediate artifact.

## Stray compiled `.js` files in `tools/src/`

`tools/src/` has both `.ts` and compiled `.js` for every module (`main.ts`/`.js`, `gbfs`, `map`, `serial`, `yaml-io`, `types`). Confirmed: `tools/index.html` loads `/src/main.ts` directly (Vite dev server transpiles on the fly), so the `.js` files aren't the live entrypoint. `tools/.gitignore` ignores `node_modules/` and `dist/` but **not** `src/*.js`, meaning these compiled files are checked into git — almost certainly stray `tsc` output from a previous build run, not intentional. Safe to delete if you're cleaning up (verify with the tool's owner first), but don't be confused into editing them thinking they're live.
