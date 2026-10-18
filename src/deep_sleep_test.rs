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
//! - 3: every board pin high-impedance (no output, input buffer or pull).
//!
//! Escape hatch: a GPIO2 (provisioning button) press wakes the chip, which
//! boots into this same window again. A power cycle clears the pad holds.

use esp_hal::delay::Delay;
use esp_hal::gpio::{
    Flex, Input, InputConfig, InputPin, Level, Output, OutputConfig, OutputPin, Pin, Pull,
    RtcPinWithResistors,
};
use esp_hal::peripherals::{GPIO2, LPWR, Peripherals};
use esp_hal::rtc_cntl::Rtc;
use esp_hal::rtc_cntl::sleep::{Ext0WakeupSource, WakeupLevel};

/// `DEEP_SLEEP_TEST` build env var, see the module docs.
pub const DEEP_SLEEP_TEST: u32 = crate::power::parse_u32(option_env!("DEEP_SLEEP_TEST"), 0);

/// No sleep for this long after reset, so probe-rs can attach (same as the
/// light-sleep `BOOT_GRACE`).
const AWAKE_WINDOW_MS: u32 = 20_000;

/// `PAD_HOLD` bits (one per RTC pad, GPIO0-21) of the board pins set up below.
/// Not GPIO2 (the wake button) or GPIO19/20 (USB).
const RTC_PAD_HOLD_MASK: u32 = 1 << 5 | 1 << 8 | 1 << 9 | 1 << 10 | 1 << 11 | 1 << 12 | 1 << 13;

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

    let rtc = Rtc::new(p.LPWR);

    if DEEP_SLEEP_TEST >= 3 {
        // No pin driven or pulled: if the board current drops, a driven pin was
        // pushing current into something.
        let _pins = (
            isolated(p.GPIO10),
            isolated(p.GPIO12),
            isolated(p.GPIO11),
            isolated(p.GPIO13),
            isolated(p.GPIO8),
            isolated(p.GPIO9),
            isolated(p.GPIO33),
            isolated(p.GPIO35),
            isolated(p.GPIO36),
            isolated(p.GPIO37),
            isolated(p.GPIO38),
            isolated(p.GPIO5),
        );
        sleep(rtc, p.GPIO2);
    }

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
    sleep(rtc, p.GPIO2)
}

fn sleep(mut rtc: Rtc<'static>, wake_button: GPIO2<'static>) -> ! {
    set_pad_holds(true);
    crate::dprintln!("DEEP_SLEEP_TEST: entering deep sleep");

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

/// Holds (or releases) the board pins' output levels and pulls through deep
/// sleep: a `PAD_HOLD` bit per used RTC pad, and ESP-IDF's
/// `gpio_deep_sleep_hold_en` (digital pad auto-hold) for GPIO33-38.
fn set_pad_holds(hold: bool) {
    let rtc_cntl = LPWR::regs();
    let rtc_pads = if hold { RTC_PAD_HOLD_MASK } else { 0 };
    rtc_cntl.pad_hold().write(|w| unsafe { w.bits(rtc_pads) });
    if hold {
        // Not `DG_PAD_FORCE_HOLD`: that freezes the SPI flash pads too while
        // the CPU still runs from flash, and it hung on the next cache miss.
        // Auto-hold latches the digital pads only once the chip is asleep.
        rtc_cntl
            .dig_iso()
            .modify(|_, w| w.dg_pad_autohold_en().set_bit());
    } else {
        rtc_cntl.dig_iso().modify(|_, w| {
            w.dg_pad_autohold_en().clear_bit();
            w.dg_pad_force_hold().clear_bit()
        });
        rtc_cntl
            .dig_iso()
            .modify(|_, w| w.clr_dg_pad_autohold().set_bit());
    }
}
