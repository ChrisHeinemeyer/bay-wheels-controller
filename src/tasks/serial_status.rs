use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

use crate::GIT_VERSION;
use crate::tasks::signals::{STATUS, SystemStatus};

/// Binary frame layout (50 bytes):
///
/// | Offset | Size | Field                                         |
/// |--------|------|-----------------------------------------------|
/// |  0     |  1   | magic = 0xAB                                  |
/// |  1     |  1   | battery_pct                                   |
/// |  2     |  1   | wifi_connected (0/1)                          |
/// |  3     |  1   | rssi (i8 bits)                                |
/// |  4     |  4   | fetch_age_secs LE  (u32::MAX = never)         |
/// |  8     |  2   | station_input LE  (StationIdx ordinal as u16) |
/// | 10     |  1   | station_input_row (0xFF = idle)               |
/// | 11     |  1   | station_input_col (0xFF = idle)               |
/// | 12     |  1   | board_id (0=Board0, 1=Board1, 2=Board2, 3=Board3) |
/// | 13     | 36   | led_rgb (12 × r,g,b)                          |
/// | 49     |  1   | XOR checksum of bytes 0–48                    |
///
/// The checksum allows the receiver to detect a false magic byte (e.g. rssi == -85 == 0xAB)
/// and re-scan for the real frame boundary.
const MAGIC: u8 = 0xAB;
const FRAME_SIZE: usize = 50;

/// Magic byte for version info frame (sent once at startup).
const VERSION_MAGIC: u8 = 0xAC;
const VERSION_STR_LEN: usize = 32;
const VERSION_FRAME_SIZE: usize = 1 + VERSION_STR_LEN + 1; // magic + version + checksum

/// Send version frame every N status frames (~10 s) so late-connecting clients can see it.
const VERSION_INTERVAL: u32 = 50;

#[embassy_executor::task]
pub async fn serial_status_task(mut serial: UsbSerialJtag<'static, esp_hal::Async>) {
    crate::dprintln!("Serial status task started!");
    let version_frame = build_version_frame();
    let mut frame_count: u32 = 0;

    loop {
        Timer::after(Duration::from_millis(200)).await;

        // Advertise version at startup and periodically for late-connecting clients.
        if frame_count % VERSION_INTERVAL == 0 {
            let _ = with_timeout(Duration::from_millis(50), serial.write_all(&version_frame)).await;
            let _ = with_timeout(Duration::from_millis(50), serial.flush()).await;
        }
        frame_count = frame_count.saturating_add(1);

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
    buf[10] = s.station_input_row.0;
    buf[11] = s.station_input_col.0;
    buf[12] = s.board_id as u8;

    for i in 0..12 {
        let (r, g, b) = s.led_states[i];
        buf[13 + i * 3] = r;
        buf[13 + i * 3 + 1] = g;
        buf[13 + i * 3 + 2] = b;
    }

    // XOR checksum of all data bytes — lets the receiver detect a false magic byte
    // (e.g. rssi == -85 dBm == 0xAB) and re-scan for the real frame boundary.
    buf[49] = buf[..49].iter().fold(0u8, |acc, b| acc ^ b);

    buf
}

fn build_version_frame() -> [u8; VERSION_FRAME_SIZE] {
    let mut buf = [0u8; VERSION_FRAME_SIZE];
    buf[0] = VERSION_MAGIC;
    let version_bytes = GIT_VERSION.as_bytes();
    let copy_len = version_bytes.len().min(VERSION_STR_LEN);
    buf[1..1 + copy_len].copy_from_slice(&version_bytes[..copy_len]);
    buf[VERSION_FRAME_SIZE - 1] = buf[..VERSION_FRAME_SIZE - 1]
        .iter()
        .fold(0u8, |acc, b| acc ^ b);
    buf
}
