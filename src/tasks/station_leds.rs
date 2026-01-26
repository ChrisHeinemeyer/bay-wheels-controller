use core::cmp::min;

use crate::spi_devices::al5887::al5887::Al5887;
use crate::spi_devices::al5887::enums::Color;
use crate::spi_devices::al5887::enums::Led;
use crate::tasks::signals::STATION_DATA_SIGNAL;
use crate::tasks::signals::STATION_SIGNAL;
use crate::tasks::signals::StationIdx;
use crate::tasks::station_parser::StationData;
use embassy_time::{Duration, Timer};
use heapless::Vec;
use rtt_target::rprintln;

#[embassy_executor::task]
pub async fn station_leds_task(mut al5887: Al5887<'static>) {
    rprintln!("Station LEDs task started!");
    al5887.init_driver().await.unwrap();
    let mut last_station = StationIdx::None;
    let mut station_data = STATION_DATA_SIGNAL.wait().await;
    rprintln!("Ready to go!");
    loop {
        let station = STATION_SIGNAL.wait().await;
        if station != last_station {
            rprintln!("Station: {:?}", station);
            let updated_station_data = STATION_DATA_SIGNAL.try_take();
            if let Some(updated_station_data) = updated_station_data {
                station_data = updated_station_data;
                rprintln!("fresh data!");
            }
            last_station = station;
            if station == StationIdx::None {
                al5887
                    .set_all_leds_brightness_color(0, Color::new(0, 0, 0))
                    .await
                    .unwrap();
            } else {
                let leds = get_leds(station, station_data);
                al5887.set_vec_led(leds).await.unwrap();
            }
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

const EBIKE_LEDS: [Led; 6] = [
    Led::Led0,
    Led::Led1,
    Led::Led2,
    Led::Led3,
    Led::Led4,
    Led::Led5,
];

const EBIKE_COLOR: Color = Color { r: 0, g: 255, b: 0 }; // Green
const MECHANICAL_BIKE_COLOR: Color = Color { r: 0, g: 0, b: 255 }; // Blue

const MECHANICAL_BIKE_LEDS: [Led; 6] = [
    Led::Led11,
    Led::Led10,
    Led::Led9,
    Led::Led8,
    Led::Led7,
    Led::Led6,
];

pub const MAX_LEDS: usize = 12;

fn get_leds(
    station_idx: StationIdx,
    station_data: [StationData; 16],
) -> Vec<(Led, Color), MAX_LEDS> {
    let mut leds = Vec::new();
    for station in station_data.iter() {
        if station.station_idx == station_idx {
            for led in 0..min(
                station.num_ebikes_available,
                MECHANICAL_BIKE_LEDS.len() as u32,
            ) {
                leds.push((EBIKE_LEDS[led as usize], EBIKE_COLOR));
            }
            for led in 0..min(
                station.num_bikes_available,
                MECHANICAL_BIKE_LEDS.len() as u32,
            ) {
                leds.push((MECHANICAL_BIKE_LEDS[led as usize], MECHANICAL_BIKE_COLOR));
            }
        }
    }
    leds
}
