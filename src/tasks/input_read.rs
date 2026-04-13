use embassy_time::{Duration, Instant, Timer};

use crate::grid::{Column, Row};
use crate::spi_devices;
use crate::stations::{BOARD_STATION_MAP, StationIdx};
use crate::tasks::signals::{BoardId, STATION_SIGNAL, STATUS};

/// Poll quickly after a touch so release/change feels responsive.
const POLL_ACTIVE_MS: u64 = 150;
/// Normal idle poll interval.
const POLL_IDLE_MS: u64 = 500;
/// How long to keep using the fast rate after the last non-None reading.
const ACTIVE_WINDOW_MS: u64 = 10_000;

#[embassy_executor::task]
pub async fn input_read_task(
    mut shift_register: spi_devices::shift_register::ShiftRegister<'static>,
    board_id: BoardId,
) {
    crate::dprintln!("Input read task started! Board ID: {:?}", board_id);
    let mut last_touch: Option<Instant> = None;

    loop {
        let value = shift_register.read().await;
        match value {
            Ok((row, col)) => {
                let station = value_to_station(row, col, board_id);
                if station != StationIdx::None {
                    last_touch = Some(Instant::now());
                }
                STATION_SIGNAL.signal(station);
                let mut status = STATUS.lock().await;
                status.station_input = station;
                status.station_input_row = row;
                status.station_input_col = col;
            }
            Err(e) => {
                crate::dprintln!("Error: {:?}", e);
            }
        }

        let in_active_window = last_touch
            .map(|t| Instant::now().duration_since(t).as_millis() < ACTIVE_WINDOW_MS)
            .unwrap_or(false);

        let poll_ms = if in_active_window {
            POLL_ACTIVE_MS
        } else {
            POLL_IDLE_MS
        };
        Timer::after(Duration::from_millis(poll_ms)).await;
    }
}

/// Maps the (row, column) from the shift register to a `StationIdx` using this
/// board's entry in `BOARD_STATION_MAP`.
/// Returns `StationIdx::None` if no input is active or the active (row, col) is
/// not listed for this board.
fn value_to_station(row: Row, col: Column, board_id: BoardId) -> StationIdx {
    if row == Row::IDLE || col == Column::IDLE {
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
