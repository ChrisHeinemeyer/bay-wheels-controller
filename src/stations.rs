use crate::tasks::signals::StationIdx;

pub static TARGET_STATIONS: &[(&str, &str, StationIdx)] = &[
    (
        "bfb90ed7-6039-4c61-9b13-fb60b1786dde",
        "McAllister St at Arguello Blvd",
        StationIdx::Station0,
    ),
    (
        "f0083331-9bf8-407f-bba2-ab00c8968db9",
        "Arguello Blvd at Edward ",
        StationIdx::Station1,
    ),
    (
        "4d2b40cb-88e2-4371-8532-b1e52e797c8c",
        "Harrison St at 17th St",
        StationIdx::Station2,
    ),
];
