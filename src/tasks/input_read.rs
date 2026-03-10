use embassy_time::{Duration, Timer};
use rtt_target::rprintln;

use crate::spi_devices;
use crate::stations::{StationIdx, BOARD_STATION_MAP};
use crate::tasks::signals::{BoardId, STATION_SIGNAL, STATUS};

#[embassy_executor::task]
pub async fn input_read_task(
    mut shift_register: spi_devices::shift_register::ShiftRegister<'static>,
    board_id: BoardId,
) {
    rprintln!("Input read task started! Board ID: {:?}", board_id);
    loop {
        let value = shift_register.read().await;
        match value {
            Ok(value) => {
                let station = value_to_station(value, board_id);
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

/// Finds the first active-low bit in the shift-register `value` and maps it to
/// a `StationIdx` using this board's entry in `BOARD_STATION_MAP`.
/// Returns `StationIdx::None` if no input is active or the active input is not
/// listed for this board.
fn value_to_station(value: u16, board_id: BoardId) -> StationIdx {
    let station_map = BOARD_STATION_MAP
        .iter()
        .find(|(id, _)| *id == board_id)
        .map(|(_, map)| *map)
        .unwrap_or(&[]);

    for i in 0u16..16 {
        if value & (1 << (15 - i)) == 0 {
            return station_map
                .iter()
                .find(|(input, _)| *input == i)
                .map(|(_, s)| *s)
                .unwrap_or(StationIdx::None);
        }
    }
    StationIdx::None
}
