use embassy_time::{Duration, Timer};

use crate::spi_devices;
use crate::stations::{BOARD_STATION_MAP, StationIdx};
use crate::tasks::signals::{BoardId, STATION_SIGNAL, STATUS};

#[embassy_executor::task]
pub async fn input_read_task(
    mut shift_register: spi_devices::shift_register::ShiftRegister<'static>,
    board_id: BoardId,
) {
    crate::dprintln!("Input read task started! Board ID: {:?}", board_id);
    loop {
        let value = shift_register.read().await;
        match value {
            Ok((first, second)) => {
                let station = value_to_station(first, second, board_id);
                STATION_SIGNAL.signal(station);
                let mut status = STATUS.lock().await;
                status.station_input = station;
                status.station_input_row = first;
                status.station_input_col = second;
            }
            Err(e) => {
                crate::dprintln!("Error: {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Maps the (row, column) from the shift register to a `StationIdx` using this
/// board's entry in `BOARD_STATION_MAP`. First u8 = row, second u8 = column.
/// Returns `StationIdx::None` if no input is active or the active (row, col) is
/// not listed for this board.
fn value_to_station(row: u8, col: u8, board_id: BoardId) -> StationIdx {
    if row == 0xFF || col == 0xFF {
        return StationIdx::None;
    }

    let station_map = BOARD_STATION_MAP
        .iter()
        .find(|(id, _)| *id == board_id)
        .map(|(_, map)| *map)
        .unwrap_or(&[]);

    station_map
        .iter()
        .find(|((r, c), _)| *r == row && *c == col)
        .map(|(_, s)| *s)
        .unwrap_or(StationIdx::Unknown)
}
