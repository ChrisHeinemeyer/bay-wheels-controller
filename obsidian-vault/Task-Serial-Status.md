---
tags: [task, serial-protocol]
---

# `serial_status_task` (`src/tasks/serial_status.rs`)

Reports the shared `STATUS` struct to whatever's listening on USB-Serial-JTAG (normally the companion web app in `web/`, via WebSerial) as a binary frame. Only spawned when the `debug-serial` feature is **off** — see [[Build-and-CI#Cargo features]] for the mutual exclusion with the text-based debug logger.

See [[Serial-Protocol]] for the full byte layout. This note covers the task's *behavior*, not the wire format.

## Connection-aware backoff

```rust
const DISCONNECTED_SLEEP_MS: u64 = 2_000;
const CONNECTED_SLEEP_MS: u64 = 300;
```

Every cycle it attempts a write with a 10ms timeout, then a flush with a 10ms timeout; `host_connected = flush_ok`. If either times out (typically because no host is actually reading the USB FIFO — e.g. the browser tab is closed), the *next* cycle sleeps 2s instead of 300ms. This avoids hammering the USB peripheral at 3.3Hz forever when nobody's listening, while still reconnecting quickly (within 2s) once a host does show up.

## Version frame

A separate, distinct-magic-byte frame (`VERSION_MAGIC = 0xAC`) carrying `GIT_VERSION` (see [[Build-and-CI#Git version embedding]]) is sent once at startup and then every `VERSION_INTERVAL = 50` status frames (~10s at the connected rate) so a browser tab that connects *after* boot still eventually sees the firmware version.

## Framing / resync

Both frame types end in an XOR checksum of all preceding bytes. The comment at `serial_status.rs:25` explains why this matters: a data byte can coincidentally equal the magic byte (e.g. `rssi == -85` is `0xAB`, same as the status frame's magic), so the receiver needs the checksum to detect a false-positive frame start and re-scan — pure magic-byte matching isn't sufficient framing on its own. See `web/src/serial-frame.ts` for the receiving side of this.
