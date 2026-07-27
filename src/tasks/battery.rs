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

/// (pack mV, percent) breakpoints approximating a 3× AA alkaline discharge curve, highest
/// voltage first. Alkaline cells hold voltage relatively flat for most of their capacity and
/// then fall off a cliff near depletion — a plain linear map (the old approach) overstates
/// capacity through the flat region and understates how little is left once voltage starts
/// sliding. Derived from a commonly-cited single-cell alkaline curve (1.50V/1.40V/1.30V/1.20V/
/// 1.10V/1.00V/0.90V at 100/83/62/42/23/8/0%), scaled ×3 for the series pack and interpolated
/// to finer steps.
const DISCHARGE_CURVE_MV: [(u16, u8); 15] = [
    (4500, 100),
    (4410, 95),
    (4320, 90),
    (4240, 85),
    (4160, 80),
    (4010, 70),
    (3870, 60),
    (3720, 50),
    (3570, 40),
    (3410, 30),
    (3240, 20),
    (3040, 10),
    (2890, 5),
    (2780, 2),
    (2700, 0),
];

fn convert_to_percentage(adc_mv: u16) -> u8 {
    // Undo the voltage divider (22 kΩ low-side, 100 kΩ high-side) to recover battery voltage.
    let voltage_div_ratio: f32 = 22.0 / (100.0 + 22.0);
    let battery_mv = adc_mv as f32 / voltage_div_ratio;

    if battery_mv >= DISCHARGE_CURVE_MV[0].0 as f32 {
        return 100;
    }
    let (min_mv, min_pct) = DISCHARGE_CURVE_MV[DISCHARGE_CURVE_MV.len() - 1];
    if battery_mv <= min_mv as f32 {
        return min_pct;
    }

    // Find the two breakpoints battery_mv falls between and linearly interpolate.
    for window in DISCHARGE_CURVE_MV.windows(2) {
        let (hi_mv, hi_pct) = window[0];
        let (lo_mv, lo_pct) = window[1];
        if battery_mv <= hi_mv as f32 && battery_mv >= lo_mv as f32 {
            let frac = (battery_mv - lo_mv as f32) / (hi_mv - lo_mv) as f32;
            let pct = lo_pct as f32 + frac * (hi_pct - lo_pct) as f32;
            return (pct + 0.5) as u8; // round to nearest without a libm dependency
        }
    }
    min_pct // unreachable given the clamps above
}
