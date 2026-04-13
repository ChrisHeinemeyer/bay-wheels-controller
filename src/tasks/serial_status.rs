use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

use crate::GIT_VERSION;
use crate::tasks::signals::{STATUS, SystemStatus};

/// Binary frame layout (51 bytes):
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
/// | 13     |  1   | tx_power (i8, quarter-dBm; divide by 4 for dBm)  |
/// | 14     | 36   | led_rgb (12 × r,g,b)                          |
/// | 50     |  1   | XOR checksum of bytes 0–49                    |
///
/// The checksum allows the receiver to detect a false magic byte (e.g. rssi == -85 == 0xAB)
/// and re-scan for the real frame boundary.
const MAGIC: u8 = 0xAB;
const FRAME_SIZE: usize = 51;

/// Magic byte for version info frame (sent once at startup).
const VERSION_MAGIC: u8 = 0xAC;
const VERSION_STR_LEN: usize = 32;
const VERSION_FRAME_SIZE: usize = 1 + VERSION_STR_LEN + 1; // magic + version + checksum

/// Send version frame every N status frames (~10 s) so late-connecting clients can see it.
const VERSION_INTERVAL: u32 = 50;

/// If a write times out the host likely isn't connected. Sleep this long before retrying
/// instead of hammering the FIFO every 300 ms.
const DISCONNECTED_SLEEP_MS: u64 = 2_000;
const CONNECTED_SLEEP_MS: u64 = 300;

#[embassy_executor::task]
pub async fn serial_status_task(mut serial: UsbSerialJtag<'static, esp_hal::Async>) {
    crate::dprintln!("Serial status task started!");
    let version_frame = build_version_frame();
    let mut frame_count: u32 = 0;
    let mut host_connected = true;

    loop {
        let sleep_ms = if host_connected {
            CONNECTED_SLEEP_MS
        } else {
            DISCONNECTED_SLEEP_MS
        };
        Timer::after(Duration::from_millis(sleep_ms)).await;

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

        // Non-blocking: if the USB FIFO doesn't drain within 10 ms no host is reading.
        // Track the outcome to adjust the sleep interval — avoid waking every 300 ms
        // when the browser tab is closed.
        let write_ok = with_timeout(Duration::from_millis(10), serial.write_all(&frame))
            .await
            .is_ok();
        let flush_ok = write_ok
            && with_timeout(Duration::from_millis(10), serial.flush())
                .await
                .is_ok();
        host_connected = flush_ok;
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
    buf[13] = s.tx_power as u8;

    for i in 0..12 {
        let (r, g, b) = s.led_states[i];
        buf[14 + i * 3] = r;
        buf[14 + i * 3 + 1] = g;
        buf[14 + i * 3 + 2] = b;
    }

    // XOR checksum of all data bytes — lets the receiver detect a false magic byte
    // (e.g. rssi == -85 dBm == 0xAB) and re-scan for the real frame boundary.
    buf[50] = buf[..50].iter().fold(0u8, |acc, b| acc ^ b);

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
