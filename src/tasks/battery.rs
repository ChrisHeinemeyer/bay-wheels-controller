use crate::tasks::signals::STATUS;
use embassy_time::{Duration, Timer};

use esp_hal::analog::adc::{Adc, AdcCalLine, AdcPin};

#[embassy_executor::task]
pub async fn battery_task(
    mut adc1: Adc<'static, esp_hal::peripherals::ADC1<'static>, esp_hal::Blocking>,
    mut pin: AdcPin<
        esp_hal::peripherals::GPIO5<'static>,
        esp_hal::peripherals::ADC1<'static>,
        AdcCalLine<esp_hal::peripherals::ADC1<'static>>,
    >,
) {
    crate::dprintln!("Battery task started!");
    loop {
        let adc_mv = nb::block!(adc1.read_oneshot(&mut pin)).unwrap_or(0);
        crate::dprintln!("Battery ADC value: {} mV", adc_mv);
        let battery_pct = convert_to_percentage(adc_mv);
        STATUS.lock().await.battery_pct = battery_pct;
        Timer::after(Duration::from_secs(5)).await;
    }
}

fn convert_to_percentage(adc_mv: u16) -> u8 {
    // Undo the voltage divider (22 kΩ low-side, 100 kΩ high-side) to recover battery voltage.
    let voltage_div_ratio: f32 = 22.0 / (100.0 + 22.0);
    let battery_mv = adc_mv as f32 / voltage_div_ratio;

    // 3× AA alkaline: ~3000 mV depleted → ~4500 mV fresh
    let min_mv = 3000.0_f32;
    let max_mv = 4500.0_f32;
    ((battery_mv - min_mv) / (max_mv - min_mv) * 100.0)
        .clamp(0.0, 100.0) as u8
}
