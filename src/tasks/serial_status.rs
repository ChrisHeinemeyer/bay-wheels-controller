use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

use crate::tasks::signals::{STATUS, SystemStatus};

/// Binary frame layout (49 bytes):
///
/// | Offset | Size | Field                                         |
/// |--------|------|-----------------------------------------------|
/// |  0     |  1   | magic = 0xAB                                  |
/// |  1     |  1   | battery_pct                                   |
/// |  2     |  1   | wifi_connected (0/1)                          |
/// |  3     |  1   | rssi (i8 bits)                                |
/// |  4     |  4   | fetch_age_secs LE  (u32::MAX = never)         |
/// |  8     |  2   | station_input LE  (StationIdx ordinal as u16) |
/// | 10     |  2   | station_input_raw LE  (raw shift register u16)|
/// | 12     | 36   | led_rgb (12 × r,g,b)                          |
/// | 48     |  1   | XOR checksum of bytes 0–47                    |
///
/// The checksum allows the receiver to detect a false magic byte (e.g. rssi == -85 == 0xAB)
/// and re-scan for the real frame boundary.
const MAGIC: u8 = 0xAB;
const FRAME_SIZE: usize = 49;

#[embassy_executor::task]
pub async fn serial_status_task(mut serial: UsbSerialJtag<'static, esp_hal::Async>) {
    crate::dprintln!("Serial status task started!");
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
            if secs > u32::MAX as u64 {
                u32::MAX
            } else {
                secs as u32
            }
        })
        .unwrap_or(u32::MAX);
    buf[4..8].copy_from_slice(&fetch_age.to_le_bytes());

    buf[8..10].copy_from_slice(&(s.station_input as u16).to_le_bytes());
    buf[10..12].copy_from_slice(&s.station_input_raw.to_le_bytes());

    for i in 0..12 {
        let (r, g, b) = s.led_states[i];
        buf[12 + i * 3] = r;
        buf[12 + i * 3 + 1] = g;
        buf[12 + i * 3 + 2] = b;
    }

    // XOR checksum of all data bytes — lets the receiver detect a false magic byte
    // (e.g. rssi == -85 dBm == 0xAB) and re-scan for the real frame boundary.
    buf[48] = buf[..48].iter().fold(0u8, |acc, b| acc ^ b);

    buf
}
