# bay-wheels-controller

ESP32-S3 firmware with Embassy, WiFi, and provisioning.

## Easy setup (web UI, no install)

A web-based flasher lets you flash firmware from your browser—no command line or drivers needed.

1. Run the web app: `cd web && npm install && npm run dev`, then open http://localhost:5173 in Chrome or Edge. (Or use the [hosted version](https://chrisheinemeyer.github.io/bay-wheels-controller/) if GitHub Pages is enabled.)
2. Connect your ESP32-S3 via USB, choose firmware (latest release or upload your own), then click "Connect & flash".

**Requirements:** Chrome or Edge on desktop (Web Serial API). Safari and Firefox are not supported.

## Flashing pre-built firmware (command line)

Pre-built firmware is available on [Releases](https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases). Download `firmware-bay-wheels-controller.bin` from the latest release.

1. Install [espflash](https://github.com/esp-rs/espflash): `cargo install espflash`
2. Connect your ESP32-S3 via USB
3. Flash the firmware:
   ```bash
   espflash flash firmware-bay-wheels-controller.bin --chip esp32s3
   ```

## Git hooks

Pre-commit runs `cargo fmt`; pre-push runs `cargo run`. Install once:

```bash
./scripts/setup-hooks.sh
```

## Building from source

Requires the [esp-rs](https://github.com/esp-rs) toolchain. See [The Rust on ESP Book](https://docs.esp-rs.org/book/) for setup.

```bash
cargo build --release
cargo espflash flash --chip esp32s3 --monitor
```
