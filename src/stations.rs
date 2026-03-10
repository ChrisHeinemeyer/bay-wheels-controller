use int_enum::IntEnum;
use strum::Display;

use crate::tasks::signals::BoardId;

#[repr(u16)]
#[derive(Copy, Clone, IntEnum, PartialEq, Display, Debug)]
pub enum StationIdx {
    McallisterArguello = 0,
    ArguelloEdward,
    HarrisonSeventeenthSt,
    ConservatoryOfFlowers,
    ArguelloGeary,
    SeventhAveCabrillo,
    EighthAveJfk,
    TurkStanyan,
    ParkerMcalister,
    FellStanyan,
    WallerShrader,
    PageMasonic,
    MlkSeventhAve,
    FrederickArguello,
    FifthAveAnza,
    SeventhAveClement,
    None = 65535,
}

/// Maps each board's shift-register bit positions (0 = first bit shifted out from MSB)
/// to the `StationIdx` that bit represents.  Bits not listed resolve to `StationIdx::None`.
///
/// Add a new `(BoardId::BoardN, &[...])` entry when a new board layout is defined.
pub static BOARD_STATION_MAP: &[(BoardId, &[(u16, StationIdx)])] = &[
    (
        BoardId::Board3,
        &[
            (0, StationIdx::McallisterArguello),
            (1, StationIdx::ArguelloEdward),
            (2, StationIdx::HarrisonSeventeenthSt),
            (3, StationIdx::ConservatoryOfFlowers),
            (4, StationIdx::ArguelloGeary),
            (5, StationIdx::SeventhAveCabrillo),
            (6, StationIdx::EighthAveJfk),
            (7, StationIdx::TurkStanyan),
            (8, StationIdx::ParkerMcalister),
            (9, StationIdx::FellStanyan),
            (10, StationIdx::WallerShrader),
            (11, StationIdx::PageMasonic),
            (12, StationIdx::MlkSeventhAve),
            (13, StationIdx::FrederickArguello),
            (14, StationIdx::FifthAveAnza),
            (15, StationIdx::SeventhAveClement),
        ],
    ),
    // (BoardId::Board0, &[ ... ]),
    // (BoardId::Board1, &[ ... ]),
    // (BoardId::Board2, &[ ... ]),
];

/// Maps GBFS station UUIDs to their `StationIdx` for the GBFS fetch task.
pub static TARGET_STATIONS: &[(&str, StationIdx)] = &[
    (
        "bfb90ed7-6039-4c61-9b13-fb60b1786dde",
        StationIdx::McallisterArguello,
    ),
    (
        "f0083331-9bf8-407f-bba2-ab00c8968db9",
        StationIdx::ArguelloEdward,
    ),
    (
        "4d2b40cb-88e2-4371-8532-b1e52e797c8c",
        StationIdx::HarrisonSeventeenthSt,
    ),
    ("1838251762103669212", StationIdx::ConservatoryOfFlowers),
    (
        "b513d838-5423-490c-bf7d-9f20771cf529",
        StationIdx::ArguelloGeary,
    ),
    (
        "2815280a-8263-4537-a944-4fdc32774fde",
        StationIdx::SeventhAveCabrillo,
    ),
    ("1838253785033265540", StationIdx::EighthAveJfk),
    (
        "9d47758e-44c7-4a16-a66a-efe7f942e7d1",
        StationIdx::TurkStanyan,
    ),
    (
        "f2c2ba49-25e1-4493-8565-5df5af66e153",
        StationIdx::ParkerMcalister,
    ),
    (
        "180e4d99-3717-4b99-8200-671af652921f",
        StationIdx::FellStanyan,
    ),
    (
        "7d15210b-55fb-4456-8281-781cfa9d5594",
        StationIdx::WallerShrader,
    ),
    (
        "b15389ba-6c9e-4c5a-bd3b-e4539b8b908c",
        StationIdx::PageMasonic,
    ),
    ("1838252908859937034", StationIdx::MlkSeventhAve),
    (
        "ec61cf51-8375-4959-9f51-b225e317abb7",
        StationIdx::FrederickArguello,
    ),
    (
        "9b4f6ef0-4568-4f52-8226-ec8432b75802",
        StationIdx::FifthAveAnza,
    ),
    (
        "df4b01d6-90bb-4faa-8437-d0391b8d115b",
        StationIdx::SeventhAveClement,
    ),
];
