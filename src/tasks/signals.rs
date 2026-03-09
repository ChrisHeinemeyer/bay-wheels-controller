use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;

use int_enum::IntEnum;
use strum::Display;

use crate::tasks::station_parser::StationData;

#[repr(u8)]
#[derive(Copy, Clone, IntEnum, PartialEq, Display, Debug)]
pub enum StationIdx {
    McallisterArguello = 0,
    ArguelloEdward = 1,
    HarrisonSeventeenthSt = 2,
    ConservatoryOfFlowers = 3,
    ArguelloGeary = 4,
    SeventhAveCabrillo = 5,
    EighthAveJfk = 6,
    TurkStanyan = 7,
    ParkerMcalister = 8,
    FellStanyan = 9,
    WallerShrader = 10,
    PageMasonic = 11,
    MlkSeventhAve = 12,
    FrederickArguello = 13,
    FifthAveAnza = 14,
    SeventhAveClement = 15,
    None = 255,
}

pub static STATION_SIGNAL: Signal<CriticalSectionRawMutex, StationIdx> = Signal::new();
pub static STATION_DATA_SIGNAL: Signal<CriticalSectionRawMutex, [StationData; 16]> = Signal::new();

/// Shared system status written by individual tasks and read by the serial status reporter.
#[derive(Copy, Clone)]
pub struct SystemStatus {
    pub battery_pct: u8,
    pub wifi_connected: bool,
    pub rssi: i8,
    /// `None` until the first successful GBFS fetch completes.
    pub last_fetch_at: Option<Instant>,
    pub station_input: StationIdx,
    /// (r, g, b) brightness values for each of the 12 LEDs (index = Led ordinal).
    pub led_states: [(u8, u8, u8); 12],
}

pub static STATUS: Mutex<CriticalSectionRawMutex, SystemStatus> = Mutex::new(SystemStatus {
    battery_pct: 100,
    wifi_connected: false,
    rssi: 0,
    last_fetch_at: None,
    station_input: StationIdx::None,
    led_states: [(0, 0, 0); 12],
});
