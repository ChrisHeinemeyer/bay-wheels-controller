use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn blink_task(
    mut pin: esp_hal::mcpwm::operator::PwmPin<
        'static,
        esp_hal::peripherals::MCPWM0<'static>,
        0,
        true,
    >,
) {
    crate::dprintln!("Blink task started!");
    loop {
        pin.set_timestamp(0);
        Timer::after(Duration::from_secs(1)).await;
        pin.set_timestamp(10);
        Timer::after(Duration::from_secs(1)).await;
    }
}
