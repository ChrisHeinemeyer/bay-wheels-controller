use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;

use crate::grid::{Column, Row};
use crate::stations::{STATION_DATA_LEN, StationIdx};
use crate::tasks::station_parser::StationData;

/// Two-bit board identifier read from GPIO37 (bit 0) and GPIO38 (bit 1) at boot.
/// Both pins are pulled up internally, so an unpopulated board reads 0b11 (Board3).
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum BoardId {
    Board0 = 0b00,
    Board1 = 0b01,
    Board2 = 0b10,
    Board3 = 0b11,
}

impl BoardId {
    /// Construct from the two GPIO levels: `bit1` = GPIO38, `bit0` = GPIO37.
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0b00 => BoardId::Board0,
            0b01 => BoardId::Board1,
            0b10 => BoardId::Board2,
            _ => BoardId::Board3,
        }
    }
}

pub static STATION_SIGNAL: Signal<CriticalSectionRawMutex, StationIdx> = Signal::new();
pub static STATION_DATA_SIGNAL: Signal<CriticalSectionRawMutex, [StationData; STATION_DATA_LEN]> =
    Signal::new();

/// Shared system status written by individual tasks and read by the serial status reporter.
#[derive(Copy, Clone)]
pub struct SystemStatus {
    pub board_id: BoardId,
    pub battery_pct: u8,
    pub wifi_connected: bool,
    pub rssi: i8,
    /// `None` until the first successful GBFS fetch completes.
    pub last_fetch_at: Option<Instant>,
    pub station_input: StationIdx,
    /// Row index (bits 0..18) from shift register; Row::IDLE = no input.
    pub station_input_row: Row,
    /// Column index (bits 18..38) from shift register; Column::IDLE = no input.
    pub station_input_col: Column,
    /// (r, g, b) brightness values for each of the 12 LEDs (index = Led ordinal).
    pub led_states: [(u8, u8, u8); 12],
}

pub static STATUS: Mutex<CriticalSectionRawMutex, SystemStatus> = Mutex::new(SystemStatus {
    board_id: BoardId::Board3,
    battery_pct: 100,
    wifi_connected: false,
    rssi: 0,
    last_fetch_at: None,
    station_input: StationIdx::None,
    station_input_row: Row::IDLE,
    station_input_col: Column::IDLE,
    led_states: [(0, 0, 0); 12],
});
