use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use rtt_target::rprintln;

use crate::tasks::signals::{STATUS, SystemStatus};

/// Binary frame layout (46 bytes):
///
/// | Offset | Size | Field                                    |
/// |--------|------|------------------------------------------|
/// |  0     |  1   | magic = 0xAB                             |
/// |  1     |  1   | battery_pct                              |
/// |  2     |  1   | wifi_connected (0/1)                     |
/// |  3     |  1   | rssi (i8 bits)                           |
/// |  4     |  4   | fetch_age_secs LE  (u32::MAX = never)    |
/// |  8     |  1   | station_input                            |
/// |  9     | 36   | led_rgb (12 × r,g,b)                     |
/// | 45     |  1   | XOR checksum of bytes 0–44               |
///
/// The checksum allows the receiver to detect a false magic byte (e.g. rssi == -85 == 0xAB)
/// and re-scan for the real frame boundary.
const MAGIC: u8 = 0xAB;
const FRAME_SIZE: usize = 46;

#[embassy_executor::task]
pub async fn serial_status_task(mut serial: UsbSerialJtag<'static, esp_hal::Async>) {
    rprintln!("Serial status task started!");
    loop {
        Timer::after(Duration::from_millis(500)).await;

        let frame = {
            let guard = STATUS.lock().await;
            build_frame(&guard)
        };

        // Non-blocking: drop the frame if the USB FIFO doesn't drain within 10 ms,
        // which means no host is reading (e.g. the browser tab is closed).
        let _ = with_timeout(Duration::from_millis(10), serial.write_all(&frame)).await;
        let _ = with_timeout(Duration::from_millis(10), serial.flush()).await;
    }
}

fn build_frame(s: &SystemStatus) -> [u8; FRAME_SIZE] {
    let mut buf = [0u8; FRAME_SIZE];

    buf[0] = MAGIC;
    buf[1] = s.battery_pct;
    buf[2] = s.wifi_connected as u8;
    buf[3] = s.rssi as u8;

    let fetch_age = s
        .last_fetch_at
        .map(|t| {
            let secs = Instant::now().duration_since(t).as_secs();
            if secs > u32::MAX as u64 { u32::MAX } else { secs as u32 }
        })
        .unwrap_or(u32::MAX);
    buf[4..8].copy_from_slice(&fetch_age.to_le_bytes());

    buf[8] = s.station_input as u8;

    for i in 0..12 {
        let (r, g, b) = s.led_states[i];
        buf[9 + i * 3]     = r;
        buf[9 + i * 3 + 1] = g;
        buf[9 + i * 3 + 2] = b;
    }

    // XOR checksum of all data bytes — lets the receiver detect a false magic byte
    // (e.g. rssi == -85 dBm == 0xAB) and re-scan for the real frame boundary.
    buf[45] = buf[..45].iter().fold(0u8, |acc, b| acc ^ b);

    buf
}
