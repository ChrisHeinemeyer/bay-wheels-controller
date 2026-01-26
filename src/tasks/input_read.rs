use embassy_time::{Duration, Timer};
use rtt_target::rprintln;

use crate::spi_devices;

#[embassy_executor::task]
pub async fn input_read_task(
    mut shift_register: spi_devices::shift_register::ShiftRegister<'static>,
) {
    rprintln!("Input read task started!");
    loop {
        let value = shift_register.read().await.unwrap();
        rprintln!("Input value: {}", value);
        let station = value_to_station(value);
        if let Some(station) = station {
            rprintln!("Station: {}", station);
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

fn value_to_station(value: u16) -> Option<u8> {
    for i in 0..16 {
        if value & (1 << (15 - i)) == 0 {
            return Some(i);
        }
    }
    return None;
}
