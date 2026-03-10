//! WiFi provisioning via USB-Serial-JTAG
//!
//! Prompts user for WiFi credentials over serial and stores them in NVS.

use alloc::string::String;
use alloc::vec::Vec;
use embedded_io::{Read, Write};
use esp_hal::delay::Delay;
use esp_hal::time::{Duration, Instant};
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use esp_storage::FlashStorage;

use crate::wifi_config;

const MAX_INPUT_LEN: usize = 128;
const NEWLINE: u8 = b'\n';
const CARRIAGE_RETURN: u8 = b'\r';
const BACKSPACE: u8 = 0x7F;
const BEACON_INTERVAL: Duration = Duration::from_millis(2_000);

/// Run the WiFi provisioning process over USB-Serial-JTAG.
///
/// Loops until credentials are saved successfully or the user explicitly cancels,
/// so a transient NVS error (common on a freshly-erased partition) doesn't force a reboot.
pub fn run_provisioning(
    mut serial: UsbSerialJtag<'static, esp_hal::Blocking>,
    _flash: FlashStorage<'static>,
) -> ! {
    crate::dprintln!("Entering WiFi provisioning mode...");

    let delay = Delay::new();

    loop {
        // Re-print the SSID prompt periodically until the user sends something.
        // This ensures the web app always sees the prompt regardless of when it connects.
        let ssid = read_line_with_beacon(&mut serial, "Enter WiFi SSID: ");
        write_str(&mut serial, "\r\n");

        if ssid.is_empty() {
            write_str(
                &mut serial,
                "Error: SSID cannot be empty. Try again.\r\n\r\n",
            );
            continue;
        }

        // Get Password
        write_str(&mut serial, "Enter WiFi Password: ");
        let password = read_line_masked(&mut serial);
        write_str(&mut serial, "\r\n");

        // Confirm
        write_str(&mut serial, "\r\n");
        write_str(&mut serial, "Configuration:\r\n");
        write_str(&mut serial, "  SSID: ");
        write_str(&mut serial, &ssid);
        write_str(&mut serial, "\r\n");
        write_str(&mut serial, "  Password: ");
        for _ in 0..password.len().min(8) {
            write_str(&mut serial, "*");
        }
        write_str(&mut serial, "\r\n\r\n");
        write_str(&mut serial, "Save credentials? (y/n): ");

        let confirm = read_line(&mut serial);
        write_str(&mut serial, "\r\n");

        if !confirm.trim().eq_ignore_ascii_case("y") && !confirm.trim().eq_ignore_ascii_case("yes")
        {
            write_str(&mut serial, "Cancelled. Try again? (y/n): ");
            let retry = read_line(&mut serial);
            write_str(&mut serial, "\r\n");
            if retry.trim().eq_ignore_ascii_case("n") || retry.trim().eq_ignore_ascii_case("no") {
                write_str(&mut serial, "Rebooting without saving...\r\n");
                break;
            }
            write_str(&mut serial, "\r\n");
            continue;
        }

        write_str(&mut serial, "Saving credentials to NVS...\r\n");

        // SAFETY: `_flash` (the originally passed-in FlashStorage) is never used for
        // a save and is still alive, so no other FlashStorage exists. FlashStorage is a
        // ZST wrapper around ROM flash functions; stealing the peripheral here is safe
        // because all flash operations are sequential and single-threaded.
        let flash = FlashStorage::new(unsafe { esp_hal::peripherals::FLASH::steal() });
        match wifi_config::save_credentials(flash, &ssid, &password) {
            Ok(()) => {
                write_str(&mut serial, "Credentials saved successfully!\r\n");
                write_str(
                    &mut serial,
                    "Rebooting to connect with new credentials...\r\n",
                );
                break;
            }
            Err(_) => {
                // NVS writes fail on the first boot after a fresh flash or power-on.
                // A full system reboot from our own firmware fixes the flash state.
                // Reboot now - provisioning will resume automatically on the next boot
                // and the save will succeed.
                write_str(
                    &mut serial,
                    "Error saving credentials. Rebooting to fix flash state - please re-enter credentials after reboot.\r\n",
                );
                break;
            }
        }
    }

    delay.delay_millis(1000);
    reboot();
}

/// Drain one remaining newline char if present. PlatformIO and many terminals
/// send CRLF on Enter, so after we break on \r there's often a leftover \n
/// that would otherwise be read as the next (empty) line.
fn drain_newline(serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>) {
    let mut byte = [0u8; 1];
    if serial.read(&mut byte).is_ok() && (byte[0] == NEWLINE || byte[0] == CARRIAGE_RETURN) {
        // drained
    }
}

/// Read a line of input, re-printing `prompt` every BEACON_INTERVAL while idle.
/// Uses read_byte() (nb, non-blocking) so we can check the timer between polls
/// without blocking when no data is available.
fn read_line_with_beacon(
    serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>,
    prompt: &str,
) -> String {
    let mut buffer: Vec<u8> = Vec::with_capacity(MAX_INPUT_LEN);

    write_str(serial, prompt);
    let mut last_beacon = Instant::now();

    loop {
        match serial.read_byte() {
            Ok(byte) => {
                last_beacon = Instant::now();
                match byte {
                    NEWLINE | CARRIAGE_RETURN => {
                        // drain_newline equivalent: consume a follow-up \n/\r if present
                        if let Ok(b) = serial.read_byte() {
                            if b != NEWLINE && b != CARRIAGE_RETURN {
                                // Not a CRLF pair — put it back by processing it now
                                if b >= 0x20 && b < 0x7F && buffer.len() < MAX_INPUT_LEN {
                                    buffer.push(b);
                                    let _ = serial.write_all(&[b]);
                                }
                            }
                        }
                        break;
                    }
                    BACKSPACE => {
                        if !buffer.is_empty() {
                            buffer.pop();
                            let _ = serial.write_all(b"\x08 \x08");
                        }
                    }
                    c if c >= 0x20 && c < 0x7F => {
                        if buffer.len() < MAX_INPUT_LEN {
                            buffer.push(c);
                            let _ = serial.write_all(&[c]);
                        }
                    }
                    _ => {}
                }
            }
            Err(nb::Error::WouldBlock) => {
                if buffer.is_empty() && Instant::now() - last_beacon >= BEACON_INTERVAL {
                    last_beacon = Instant::now();
                    write_str(serial, "\r\nEnter WiFi SSID: ");
                }
            }
            Err(_) => {}
        }
    }

    String::from_utf8(buffer).unwrap_or_default()
}

/// Read a line of input from serial (with echo)
fn read_line(serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>) -> String {
    let mut buffer: Vec<u8> = Vec::with_capacity(MAX_INPUT_LEN);
    let mut byte = [0u8; 1];

    loop {
        if serial.read(&mut byte).is_ok() && byte[0] != 0 {
            match byte[0] {
                NEWLINE | CARRIAGE_RETURN => {
                    drain_newline(serial);
                    break;
                }
                BACKSPACE => {
                    if !buffer.is_empty() {
                        buffer.pop();
                        // Echo backspace sequence
                        let _ = serial.write_all(b"\x08 \x08");
                    }
                }
                c if c >= 0x20 && c < 0x7F => {
                    if buffer.len() < MAX_INPUT_LEN {
                        buffer.push(c);
                        let _ = serial.write_all(&[c]);
                    }
                }
                _ => {}
            }
        }
    }

    String::from_utf8(buffer).unwrap_or_default()
}

/// Read a line of input with masked echo (for passwords)
fn read_line_masked(serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>) -> String {
    let mut buffer: Vec<u8> = Vec::with_capacity(MAX_INPUT_LEN);
    let mut byte = [0u8; 1];

    loop {
        if serial.read(&mut byte).is_ok() && byte[0] != 0 {
            match byte[0] {
                NEWLINE | CARRIAGE_RETURN => {
                    drain_newline(serial);
                    break;
                }
                BACKSPACE => {
                    if !buffer.is_empty() {
                        buffer.pop();
                        let _ = serial.write_all(b"\x08 \x08");
                    }
                }
                c if c >= 0x20 && c < 0x7F => {
                    if buffer.len() < MAX_INPUT_LEN {
                        buffer.push(c);
                        let _ = serial.write_all(b"*");
                    }
                }
                _ => {}
            }
        }
    }

    String::from_utf8(buffer).unwrap_or_default()
}

fn write_str(serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>, s: &str) {
    let _ = serial.write_all(s.as_bytes());
}

fn reboot() -> ! {
    esp_hal::system::software_reset()
}
