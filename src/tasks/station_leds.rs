use crate::spi_devices::al5887::al5887::Al5887;
use crate::spi_devices::al5887::enums::Color;
use crate::spi_devices::al5887::enums::Led;
use alloc::vec;
use alloc::vec::Vec;
use embassy_time::{Duration, Timer};
use rtt_target::rprintln;

#[embassy_executor::task]
pub async fn station_leds_task(mut al5887: Al5887<'static>) {
    rprintln!("Station LEDs task started!");
    al5887.init_driver().await.unwrap();
    loop {
        for i in 0..12 {
            al5887
                .set_led_brightness_color(Led::try_from(i).unwrap(), 30, Color::new(255, 0, 0))
                .await
                .unwrap();
            let mut v: Vec<Led> = Vec::new();
            for j in 0..12 {
                if j != i {
                    v.push(Led::try_from(j).unwrap());
                }
            }
            al5887
                .set_vec_led_brightness_color(v, 10, Color::new(255, 255, 255))
                .await
                .unwrap();
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}
