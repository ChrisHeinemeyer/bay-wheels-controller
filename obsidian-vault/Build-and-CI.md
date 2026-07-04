---
tags: [build, ci]
---

# Build, CI, and Repo Layout

## Repo layout

```
src/                  firmware (this vault documents src/ in depth)
web/                  browser flasher + live status monitor (Vite/TS) — see Companion-Tools
tools/                browser board-mapping authoring tool (Vite/TS) — see Companion-Tools
build.rs              codegen + linker helper — see below
Cargo.toml            firmware crate manifest — see Cargo features / Profiles below
.githooks/            pre-commit (fmt + lint-staged), pre-push (conditional release build)
.github/workflows/    CI: tag-triggered firmware Release build, web deploy to GitHub Pages
obsidian-vault/        ← this vault (not yet committed/gitignored as of when this was written)
```

## Cargo features (`Cargo.toml`)

- **`debug-serial`** — routes `dprintln!` to USB-Serial-JTAG (blocking) instead of RTT, and *disables spawning* [[Task-Serial-Status]] (mutually exclusive: both want the same USB peripheral for different purposes — text debug log vs. binary status frame). Useful for flashing/debugging without a JTAG probe attached; not meant for the shipped firmware since it forfeits the web UI's live status.
- **`use-env`** — loads `SSID`/`PASSWORD` from a `.env` file at *compile* time (via `build.rs`) instead of from NVS at runtime. See [[Provisioning-and-Config#Two credential paths at compile time]] — don't ship a `use-env` binary, it embeds the WiFi password.

Neither feature is on by default; the default build path is: NVS credentials + RTT logging + `serial_status_task` spawned.

## Profiles

```toml
[profile.dev]
opt-level = 1          # plain debug is "too slow" per the comment
debug = true

[profile.release]
codegen-units = 1
debug = 2               # full debug info even in release (for stack traces / addr2line)
debug-assertions = false
incremental = false
lto = 'fat'
opt-level = 's'          # optimize for size, not speed
overflow-checks = false  # integer overflow wraps silently, doesn't panic — see Open-Questions
```

`opt-level = 's'` (size) plus `lto = 'fat'` + `codegen-units = 1` is a common embedded combination to fit flash budget, but it's a deliberate size-over-speed tradeoff — see [[Power-Management]] for why that might be worth revisiting (runtime cycle count affects how long the CPU stays awake per task wakeup, which affects battery life independent of flash size).

## Codegen (`build.rs`)

Three independent jobs run on every build:
1. **`embed_git_version`** — runs `git describe --always --dirty --tags`, embeds the result as `GIT_VERSION` env var, consumed by `lib.rs` (`pub const GIT_VERSION`) and baked into the app descriptor (`esp_bootloader_esp_idf::esp_app_desc!` in `main.rs`) and the serial version frame ([[Serial-Protocol]]). Reruns on `.git/HEAD`/`.git/index` changes — i.e. on every commit, not just tag pushes.
2. **`generate_station_ids_ts`** — hand-rolled text parser (not `syn`) that extracts `TARGET_STATIONS` tuples straight from `src/stations.rs` source text and emits `web/src/generated/station-ids.ts`. See [[Data-Model#Generated artifact]]. Only writes the file if content actually changed, to avoid spurious Vite rebuilds during `web/` dev.
3. **`load_env_file`** (only if `use-env` feature is on) — reads `.env`, panics with a helpful message if missing.

Plus `linker_be_nice()` — intercepts linker errors for common esp-hal setup mistakes (missing `linkall.x`, missing `esp-rtos`/scheduler init, missing `esp-alloc` compat feature) and prints a human-readable hint instead of a raw undefined-symbol error.

## Git version embedding

`GIT_VERSION` (e.g. `v1.0.1-abe59403-dirty`) flows through three places: the app descriptor (visible via `esptool.py image-info` / the web flasher's "Flash tab"), `dprintln!` at startup, and the serial version frame — so you can always identify exactly what's running on a board three different ways (image inspection, debug log, or live serial query) without needing to trust a label.

## CI (`.github/workflows/`)

- **Release** (tag push `v*`): builds firmware for `esp32s3` using the `esp-rs/xtensa-toolchain` action, presumably uploads the `.bin` as a release asset (the workflow continues past what was read here — check the full file for the upload step).
- **Deploy web setup**: two triggers — (a) direct push to `main` touching `web/**`, deploys immediately; (b) `workflow_run` after Release completes, so a tag push updates both the firmware release *and* the hosted web flasher's bundled firmware binary and GBFS station data in the same logical release. Note it re-fetches `station_information.json` live from Lyft at deploy time (not committed), so the hosted map's station list can silently drift from what `TARGET_STATIONS` in the firmware knows about if Lyft adds/removes/renames stations between deploys.

## Local build + flash to real hardware (probe-rs, via `cargo run`)

This is the fast inner-loop way to get firmware onto a physical board plugged in over USB and watch its live RTT log — distinct from the README's `cargo espflash flash --chip esp32s3 --monitor` (which uses `espflash` instead of `probe-rs`; both work, but this repo's `.cargo/config.toml` wires `cargo run` to the `probe-rs` path by default).

```bash
. $HOME/export-esp.sh              # sourced, not run — puts the Xtensa esp-rs toolchain on PATH
cargo run --release --features use-env
```

- **`. $HOME/export-esp.sh`** — a local (not repo-committed) environment script from the esp-rs toolchain install (see [The Rust on ESP Book](https://docs.esp-rs.org/book/)) that puts the `xtensa-esp32s3-none-elf` Rust toolchain and its LLVM fork on `PATH`. Must be sourced (`.`/`source`), not executed, or the exports won't reach your shell. Needed once per terminal session before any `cargo build`/`run`/`check` against this crate's `xtensa-esp32s3-none-elf` target (see `.cargo/config.toml`).
- **`--features use-env`** — bypasses the NVS/serial provisioning flow entirely by baking `SSID`/`PASSWORD` from a local `.env` file (copy `.env.dist` → `.env` first) directly into the binary at compile time. Fast for iterating on a known dev network; see [[Provisioning-and-Config#Two credential paths at compile time]] for why this must never be the flashed feature set for a real deployment (the WiFi password ends up in the binary in plaintext).
- **Why `cargo run` flashes at all**: `.cargo/config.toml` sets `runner = "probe-rs run --chip=esp32s3 --preverify --always-print-stacktrace --no-location --catch-hardfault"` for the `xtensa-esp32s3-none-elf` target, so `cargo run` builds, flashes over the board's built-in USB-Serial-JTAG (no external debug probe needed on the S3), and then attaches and streams the RTT log (`dprintln!` output, since this is a non-`debug-serial` build) live in the terminal until you stop it (`Ctrl+C`, or `pkill -f "probe-rs run"` if it was launched in the background).
- The board enumerates as a USB-serial device (e.g. `/dev/tty.usbmodemXXX` on macOS) — that's the same USB-Serial-JTAG peripheral [[Task-Serial-Status]]/[[Provisioning-and-Config]] multiplex at runtime, but at the OS/tooling level it's independent of and unrelated to the RTT channel `probe-rs` uses for logs.
- To exercise the WiFi provisioning wizard instead of `use-env`, flash without that feature (`cargo run --release`) — first boot with no NVS credentials stored drops straight into [[Provisioning-and-Config]]'s serial wizard.

## Git hooks (`.githooks/`, installed via `./scripts/setup-hooks.sh`)

- **pre-commit**: `cargo fmt` + `npx lint-staged` (presumably Prettier for the TS apps, per the README).
- **pre-push**: only runs `cargo build --release` if `src/` differs between local and the remote branch (or if pushing a brand-new branch) — a cheap guard against pushing firmware that doesn't compile, without forcing a slow release build on every push that only touches `web/`/`tools/`/docs.
