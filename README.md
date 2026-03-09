# bay-wheels-controller

ESP32-S3 firmware with Embassy, WiFi, and provisioning.

## Flashing pre-built firmware

Pre-built firmware is available on [Releases](https://github.com/ChrisHeinemeyer/bay-wheels-controller/releases). Download `firmware-bay-wheels-controller.bin` from the latest release.

1. Install [espflash](https://github.com/esp-rs/espflash): `cargo install espflash`
2. Connect your ESP32-S3 via USB
3. Flash the firmware:
   ```bash
   espflash flash firmware-bay-wheels-controller.bin --chip esp32s3
   ```

## Building from source

Requires the [esp-rs](https://github.com/esp-rs) toolchain. See [The Rust on ESP Book](https://docs.esp-rs.org/book/) for setup.

```bash
cargo build --release
cargo espflash flash --chip esp32s3 --monitor
```
