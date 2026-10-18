//! Board-only current test, from the `DEEP_SLEEP_TEST` build env var (default
//! 0: off). When set, `main` hands over all peripherals here right after
//! `esp_hal::init`, and the firmware never starts WiFi or any task: it waits
//! out a no-sleep window, drives every board pin to a defined level, holds the
//! pads and deep-sleeps. The ESP32-S3 then draws ~10µA, so the PPK2 reads the
//! rest of the board (regulator, AL5887, dividers, pull-ups, leakage). See
//! Sleep-Floor-Plan.md step 1 in the Obsidian vault.
//!
//! - 1: pins at the levels the running firmware leaves them in while idle
//!   (AL5887 LED_EN and RST_N high). The AL5887 is never initialised, so it sits
//!   in standby (CHIP_EN register bit 0, its power-on value).
//! - 2: as 1, but LED_EN low too: AL5887 in hardware shutdown.
//!
//! Escape hatch: a GPIO2 (provisioning button) press wakes the chip, which
//! boots into this same window again. A power cycle clears the pad holds.

use esp_hal::delay::Delay;
use esp_hal::gpio::{
    Flex, Input, InputConfig, InputPin, Level, Output, OutputConfig, OutputPin, Pin, Pull,
    RtcPinWithResistors,
};
use esp_hal::peripherals::{LPWR, Peripherals};
use esp_hal::rtc_cntl::Rtc;
use esp_hal::rtc_cntl::sleep::{Ext0WakeupSource, WakeupLevel};

/// `DEEP_SLEEP_TEST` build env var, see the module docs.
pub const DEEP_SLEEP_TEST: u32 = crate::power::parse_u32(option_env!("DEEP_SLEEP_TEST"), 0);

/// No sleep for this long after reset, so probe-rs can attach (same as the
/// light-sleep `BOOT_GRACE`).
const AWAKE_WINDOW_MS: u32 = 20_000;

/// RTC GPIO number of the GPIO2 wake button, kept out of the pad hold.
const WAKE_BUTTON_RTC_GPIO: u32 = 2;
/// GPIO0-21 are the RTC pads, one `PAD_HOLD` bit each.
const RTC_PAD_COUNT: u32 = 22;

pub fn run(p: Peripherals) -> ! {
    // Holds survive a deep-sleep wake (but not a power cycle) and would freeze
    // every pad at its old level for the whole window. Release them first.
    set_pad_holds(false);

    crate::dprintln!(
        "DEEP_SLEEP_TEST={}: deep sleep in {}s (GPIO2 wakes into this window again)",
        DEEP_SLEEP_TEST,
        AWAKE_WINDOW_MS / 1000
    );
    Delay::new().delay_millis(AWAKE_WINDOW_MS);

    let mut rtc = Rtc::new(p.LPWR);

    // Same pins and idle levels as `main.rs` sets up. The drivers must stay
    // alive (not dropped) until the chip is asleep.
    let _pins = (
        // AL5887 (SPI3): CS idles high, SCK low (mode 0), MOSI low, MISO input.
        output_high(p.GPIO10),
        output_low(p.GPIO12),
        output_low(p.GPIO11),
        input(p.GPIO13),
        // LED_EN, RST_N.
        output(
            p.GPIO8,
            if DEEP_SLEEP_TEST >= 2 {
                Level::Low
            } else {
                Level::High
            },
        ),
        output_high(p.GPIO9),
        // Shift registers (SPI2): CS idles low, SCK high (mode 2), MISO input.
        output_low(p.GPIO33),
        output_high(p.GPIO35),
        input(p.GPIO36),
        // Board-ID straps, pulled up like at boot.
        input_pulled_up(p.GPIO37),
        input_pulled_up(p.GPIO38),
        // Battery divider: no digital input buffer on its mid-rail voltage,
        // as when the ADC owns it.
        isolated(p.GPIO5),
    );

    set_pad_holds(true);
    crate::dprintln!("DEEP_SLEEP_TEST: entering deep sleep");

    let wake_button = p.GPIO2;
    // See `power::init`: the RTC-domain pull-up has to be set separately.
    wake_button.rtcio_pullup(true);
    let wake = Ext0WakeupSource::new(wake_button, WakeupLevel::Low);
    rtc.sleep_deep(&[&wake])
}

fn output<'d>(pin: impl OutputPin + 'd, level: Level) -> Output<'d> {
    Output::new(pin, level, OutputConfig::default())
}

fn output_high<'d>(pin: impl OutputPin + 'd) -> Output<'d> {
    output(pin, Level::High)
}

fn output_low<'d>(pin: impl OutputPin + 'd) -> Output<'d> {
    output(pin, Level::Low)
}

fn input<'d>(pin: impl InputPin + 'd) -> Input<'d> {
    Input::new(pin, InputConfig::default())
}

fn input_pulled_up<'d>(pin: impl InputPin + 'd) -> Input<'d> {
    Input::new(pin, InputConfig::default().with_pull(Pull::Up))
}

fn isolated<'d>(pin: impl Pin + 'd) -> Flex<'d> {
    let mut flex = Flex::new(pin);
    flex.set_input_enable(false);
    flex.set_output_enable(false);
    flex
}

/// Holds (or releases) every pad's current output level and pull through
/// deep sleep, except the GPIO2 wake button. ESP-IDF's `gpio_force_hold_all`:
/// one `PAD_HOLD` bit per RTC pad, and `DG_PAD_FORCE_HOLD` for the digital
/// pads (GPIO33-38 here).
fn set_pad_holds(hold: bool) {
    let rtc_cntl = LPWR::regs();
    let rtc_pads = if hold {
        ((1 << RTC_PAD_COUNT) - 1) & !(1 << WAKE_BUTTON_RTC_GPIO)
    } else {
        0
    };
    rtc_cntl.pad_hold().write(|w| unsafe { w.bits(rtc_pads) });
    rtc_cntl.dig_iso().modify(|_, w| {
        w.dg_pad_force_hold().bit(hold);
        w.dg_pad_force_unhold().bit(!hold)
    });
}
