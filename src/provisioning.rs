//! WiFi provisioning via USB-Serial-JTAG
//!
//! Prompts user for WiFi credentials over serial and stores them in NVS.

use alloc::string::String;
use alloc::vec::Vec;
use embedded_io::{Read, Write};
use esp_hal::delay::Delay;
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use esp_storage::FlashStorage;
use rtt_target::rprintln;

use crate::wifi_config;

const MAX_INPUT_LEN: usize = 128;
const NEWLINE: u8 = b'\n';
const CARRIAGE_RETURN: u8 = b'\r';
const BACKSPACE: u8 = 0x7F;

/// Run the WiFi provisioning process over USB-Serial-JTAG
pub fn run_provisioning(
    mut serial: UsbSerialJtag<'static, esp_hal::Blocking>,
    flash: FlashStorage<'static>,
) -> ! {
    rprintln!("Entering WiFi provisioning mode...");

    let delay = Delay::new();

    // Give user time to connect terminal
    for i in (1..=5).rev() {
        write_str(&mut serial, "\r\n");
        write_str(
            &mut serial,
            "WiFi Provisioning Mode - Connect your terminal (",
        );
        write_num(&mut serial, i);
        write_str(&mut serial, "s)...\r\n");
        delay.delay_millis(1000);
    }

    write_str(&mut serial, "\r\n");
    write_str(&mut serial, "========================================\r\n");
    write_str(&mut serial, "       ESP32-S3 WiFi Provisioning       \r\n");
    write_str(&mut serial, "========================================\r\n");
    write_str(&mut serial, "\r\n");

    // Get SSID
    write_str(&mut serial, "Enter WiFi SSID: ");
    let ssid = read_line(&mut serial);
    write_str(&mut serial, "\r\n");

    if ssid.is_empty() {
        write_str(&mut serial, "Error: SSID cannot be empty. Rebooting...\r\n");
        delay.delay_millis(1000);
        reboot();
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

    if confirm.trim().eq_ignore_ascii_case("y") || confirm.trim().eq_ignore_ascii_case("yes") {
        write_str(&mut serial, "Saving credentials to NVS...\r\n");

        match wifi_config::save_credentials(flash, &ssid, &password) {
            Ok(()) => {
                write_str(&mut serial, "Credentials saved successfully!\r\n");
                write_str(
                    &mut serial,
                    "Rebooting to connect with new credentials...\r\n",
                );
            }
            Err(e) => {
                write_str(&mut serial, "Error saving credentials: ");
                write_str(
                    &mut serial,
                    match e {
                        wifi_config::WifiConfigError::NvsError => "NVS error",
                        wifi_config::WifiConfigError::NotFound => "Not found",
                    },
                );
                write_str(&mut serial, "\r\n");
            }
        }
    } else {
        write_str(&mut serial, "Cancelled. Rebooting without saving...\r\n");
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

fn write_num(serial: &mut UsbSerialJtag<'static, esp_hal::Blocking>, n: u32) {
    let mut buf = [0u8; 10];
    let s = format_num(n, &mut buf);
    let _ = serial.write_all(s.as_bytes());
}

fn format_num(mut n: u32, buf: &mut [u8; 10]) -> &str {
    if n == 0 {
        return "0";
    }
    let mut i = buf.len();
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

fn reboot() -> ! {
    esp_hal::system::software_reset()
}
