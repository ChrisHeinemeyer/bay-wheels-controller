use embassy_time::{Duration, Timer};
use rtt_target::rprintln;

use crate::spi_devices;
use crate::tasks::signals::{STATION_SIGNAL, STATUS, StationIdx};

#[embassy_executor::task]
pub async fn input_read_task(
    mut shift_register: spi_devices::shift_register::ShiftRegister<'static>,
) {
    rprintln!("Input read task started!");
    loop {
        let value = shift_register.read().await;
        match value {
            Ok(value) => {
                let station = value_to_station(value);
                STATION_SIGNAL.signal(station);
                STATUS.lock().await.station_input = station;
            }
            Err(e) => {
                rprintln!("Error: {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

fn value_to_station(value: u16) -> StationIdx {
    for i in 0..16 {
        if value & (1 << (15 - i)) == 0 {
            return StationIdx::try_from(i).unwrap();
        }
    }
    return StationIdx::None;
}
