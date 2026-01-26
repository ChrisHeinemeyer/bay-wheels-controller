use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use int_enum::IntEnum;
use strum::Display;

use crate::tasks::station_parser::StationData;
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
