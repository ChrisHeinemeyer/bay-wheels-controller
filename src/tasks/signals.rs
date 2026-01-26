use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use int_enum::IntEnum;
use strum::Display;

use crate::tasks::station_parser::StationData;
#[derive(Copy, Clone, IntEnum, PartialEq, Display, Debug)]
pub enum StationIdx {
    Station0 = 0,
    Station1 = 1,
    Station2 = 2,
    Station3 = 3,
    Station4 = 4,
    Station5 = 5,
    Station6 = 6,
    Station7 = 7,
    Station8 = 8,
    Station9 = 9,
    Station10 = 10,
    Station11 = 11,
    Station12 = 12,
    Station13 = 13,
    Station14 = 14,
    Station15 = 15,
    None = 255,
}

pub static STATION_SIGNAL: Signal<CriticalSectionRawMutex, StationIdx> = Signal::new();
pub static STATION_DATA_SIGNAL: Signal<CriticalSectionRawMutex, [StationData; 16]> = Signal::new();
