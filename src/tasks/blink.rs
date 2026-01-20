use embassy_time::{Duration, Timer};
use esp_hal::gpio;
use rtt_target::rprintln;

#[embassy_executor::task]
pub async fn blink_task(mut pin: gpio::Output<'static>) {
    rprintln!("Blink task started!");
    loop {
        pin.toggle();
        Timer::after(Duration::from_secs(1)).await;
    }
}
