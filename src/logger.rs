//! Optional USB-serial debug logger, enabled by the `debug-serial` Cargo feature.
//!
//! When the feature is active, [`init`] must be called with the blocking
//! `UsbSerialJtag` peripheral before any task spawning.  After that every
//! `dprintln!` invocation writes a UTF-8 line to the serial port instead of
//! RTT, which is useful when flashing without a JTAG probe attached.
//!
//! When the feature is *not* active this entire module is empty and the
//! `dprintln!` macro delegates to `rtt_target::rprintln!` as before.

#[cfg(feature = "debug-serial")]
mod inner {
    use core::cell::UnsafeCell;

    use embedded_io::Write as _;
    use esp_hal::{Blocking, usb_serial_jtag::UsbSerialJtag};

    // `UnsafeCell` gives interior mutability without `static mut`.
    // `Sync` is implemented manually; soundness is guaranteed because every
    // access is wrapped in `critical_section::with`, which disables interrupts
    // and prevents the embassy executor from switching tasks.
    struct SerialCell(UnsafeCell<Option<UsbSerialJtag<'static, Blocking>>>);
    unsafe impl Sync for SerialCell {}

    static SERIAL: SerialCell = SerialCell(UnsafeCell::new(None));

    /// Initialise the serial logger.  Call once from `main` before spawning tasks.
    pub fn init(serial: UsbSerialJtag<'static, Blocking>) {
        critical_section::with(|_| {
            // SAFETY: interrupts disabled; no other accessor can exist.
            unsafe { *SERIAL.0.get() = Some(serial) };
        });
    }

    struct Sink<'a>(&'a mut UsbSerialJtag<'static, Blocking>);

    impl core::fmt::Write for Sink<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_all(s.as_bytes());
            Ok(())
        }
    }

    /// Write a formatted message followed by `\n` to the serial port.
    /// Called exclusively by the `dprintln!` macro.
    pub fn _print(args: core::fmt::Arguments<'_>) {
        critical_section::with(|_| {
            // SAFETY: interrupts disabled; no other accessor can exist.
            if let Some(serial) = unsafe { (*SERIAL.0.get()).as_mut() } {
                let _ = core::fmt::write(&mut Sink(serial), args);
                let _ = serial.write_all(b"\n");
                let _ = serial.flush();
            }
        });
    }
}

#[cfg(feature = "debug-serial")]
pub use inner::{_print, init};
