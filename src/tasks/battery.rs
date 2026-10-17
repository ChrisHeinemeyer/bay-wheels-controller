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
        // Uptime lets a host compare firmware time against its own timestamps
        // (light-sleep clock compensation check). Skipped while light-sleeping:
        // nobody can read RTT then, and these lines would fill the 1 KB buffer
        // and crowd out the sleep stats printed on wake.
        if !crate::power::SLEEPING.load(core::sync::atomic::Ordering::Relaxed) {
            crate::dprintln!(
                "Battery ADC value: {} mV (uptime {}ms)",
                adc_mv,
                embassy_time::Instant::now().as_millis()
            );
        }
        let battery_pct = convert_to_percentage(adc_mv);
        {
            let mut status = STATUS.lock().await;
            status.battery_pct = battery_pct;
            status.battery_adc_mv = adc_mv;
        }
        Timer::after(Duration::from_secs(30)).await;
    }
}

/// (pack mV, percent) breakpoints for a 3× AA alkaline pack on this board, highest voltage
/// first. Alkaline cells hold voltage relatively flat for most of their capacity and then fall
/// off a cliff near depletion, so a straight-line map overstates what is left.
///
/// The bottom half (3955 mV and below) is measured: Board3 on v1.3.1 ran from 3955 mV to its
/// last report at 3330 mV (1.11V/cell) over 88.6h at a constant duty cycle, so elapsed time
/// tracks charge drawn (log server, 2026-10-13 to 2026-10-17). Voltages are resting values;
/// samples taken during a radio TX burst read ~130 mV lower. 0% is where the board stopped
/// reporting, not where the cells are chemically empty. The top half keeps the shape of a
/// commonly-cited single-cell alkaline curve (×3 for the series pack), rescaled to 55-100%.
/// The 55% split is an estimate: no fresh-cell data yet.
const DISCHARGE_CURVE_MV: [(u16, u8); 16] = [
    (4500, 100),
    (4410, 93),
    (4320, 87),
    (4240, 80),
    (4160, 74),
    (4010, 60),
    (3955, 55),
    (3925, 50),
    (3885, 44),
    (3820, 33),
    (3760, 28),
    (3660, 17),
    (3565, 11),
    (3490, 6),
    (3405, 3),
    (3330, 0),
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
