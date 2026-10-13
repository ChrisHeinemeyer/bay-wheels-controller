#![no_std]
// Inline asm: `waiti` in power.rs's idle hook and the register-window spill in
// watchdog.rs's backtrace; same feature esp-hal/esp-rtos enable.
#![cfg_attr(target_arch = "xtensa", feature(asm_experimental_arch))]

extern crate alloc;

/// Git version (tag + commit + dirty) embedded at build time, e.g. `v1.0.1-abe59403-dirty`.
pub const GIT_VERSION: &str = env!("GIT_VERSION");

pub mod grid;
pub mod logger;
pub mod network;
pub mod power;
pub mod provisioning;
pub mod spi_devices;
pub mod stations;
pub mod tasks;
pub mod watchdog;
pub mod wifi;
pub mod wifi_config;

/// Logging macro that routes to USB serial (feature `debug-serial`) or RTT.
///
/// Usage is identical to `rprintln!` / `println!`.
///
/// * **`debug-serial` on** — writes a text line to the USB serial port via
///   [`logger::init`].  `serial_status_task` must **not** be spawned; see
///   `main.rs` for the conditional.
/// * **`debug-serial` off** — delegates to `rtt_target::rprintln!`, the
///   original behaviour.
#[macro_export]
macro_rules! dprintln {
    ($($arg:tt)*) => {{
        #[cfg(feature = "debug-serial")]
        $crate::logger::_print(format_args!($($arg)*));
        #[cfg(not(feature = "debug-serial"))]
        rtt_target::rprintln!($($arg)*);
    }};
}
