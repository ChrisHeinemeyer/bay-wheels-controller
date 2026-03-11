use core::cmp::min;

use crate::spi_devices::al5887::al5887::Al5887;
use crate::spi_devices::al5887::enums::Color;
use crate::spi_devices::al5887::enums::Led;
use crate::stations::{STATION_DATA_LEN, StationIdx};
use crate::tasks::signals::STATION_DATA_SIGNAL;
use crate::tasks::signals::STATION_SIGNAL;
use crate::tasks::signals::STATUS;
use crate::tasks::station_parser::StationData;
use embassy_time::{Duration, Timer};
use heapless::Vec;

#[embassy_executor::task]
pub async fn station_leds_task(mut al5887: Al5887<'static>) {
    crate::dprintln!("Station LEDs task started!");
    al5887.init_driver().await.unwrap();
    let mut last_station = StationIdx::None;
    let mut station_data = STATION_DATA_SIGNAL.wait().await;
    crate::dprintln!("Ready to go!");
    loop {
        let station = STATION_SIGNAL.wait().await;
        if station != last_station {
            crate::dprintln!("Station: {:?}", station);
            let updated_station_data = STATION_DATA_SIGNAL.try_take();
            if let Some(updated_station_data) = updated_station_data {
                station_data = updated_station_data;
                crate::dprintln!("fresh data!");
            }
            last_station = station;
            if station == StationIdx::None {
                STATUS.lock().await.led_states = [(0, 0, 0); 12];
                al5887
                    .set_all_leds_brightness_color(0, Color::new(0, 0, 0))
                    .await
                    .unwrap();
            } else {
                let leds = get_leds(station, &station_data);
                {
                    let mut guard = STATUS.lock().await;
                    guard.led_states = [(0, 0, 0); 12];
                    for (led, color) in leds.iter() {
                        guard.led_states[*led as usize] = (color.r, color.g, color.b);
                    }
                }
                al5887.set_vec_led(leds).await.unwrap();
            }
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

const EBIKE_LEDS: [Led; 5] = [
    Led::Led1,
    Led::Led2,
    Led::Led3,
    Led::Led4,
    Led::Led5,
];

const EBIKE_COLOR: Color = Color { r: 0, g: 255, b: 0 }; // Green
const MECHANICAL_BIKE_COLOR: Color = Color { r: 0, g: 0, b: 255 }; // Blue

const MECHANICAL_BIKE_LEDS: [Led; 5] = [
    Led::Led10,
    Led::Led9,
    Led::Led8,
    Led::Led7,
    Led::Led0,
];

const STATION_EMPTY_LEDS: [Led; 1] = [Led::Led6];
const STATION_EMPTY_COLOR: Color = Color { r: 255, g: 0, b: 0 }; // Red

pub const MAX_LEDS: usize = 12;

fn get_leds(
    station_idx: StationIdx,
    station_data: &[StationData; STATION_DATA_LEN],
) -> Vec<(Led, Color), MAX_LEDS> {
    let mut leds = Vec::new();
    // Index directly by ordinal — O(1) lookup.
    let station = &station_data[station_idx as usize];
    if station.num_ebikes_available == 0 && station.num_bikes_available == 0 {
        for &led in STATION_EMPTY_LEDS.iter() {
            if leds.push((led, STATION_EMPTY_COLOR)).is_err() {
                crate::dprintln!("Error pushing station empty led");
            }
        }
    } else {
        for led in 0..min(station.num_ebikes_available as usize, EBIKE_LEDS.len()) {
            if leds.push((EBIKE_LEDS[led], EBIKE_COLOR)).is_err() {
                crate::dprintln!("Error pushing ebike led");
            }
        }
        for led in 0..min(
            station.num_bikes_available as usize,
            MECHANICAL_BIKE_LEDS.len(),
        ) {
            if leds.push((MECHANICAL_BIKE_LEDS[led], MECHANICAL_BIKE_COLOR)).is_err() {
                crate::dprintln!("Error pushing mech bike led");
            }
        }
    }
    leds
}
