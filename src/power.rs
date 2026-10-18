//! CPU light-sleep during scheduler idle gaps, gated by the `light-sleep` Cargo
//! feature.
//!
//! Only sleeps while the WiFi radio is fully stopped: [`crate::tasks::fetch`]
//! holds [`RADIO_ACTIVE`] for its whole start→fetch→teardown window, and this
//! Rust WiFi/RTOS stack has no `esp_pm`-equivalent PM-lock coordination, so a
//! light-sleep nap while the radio is up would break the connection.
//!
//! Light sleep gates XTAL, which stops both esp-rtos's clock (SYSTIMER unit 0,
//! via `esp_hal::time::Instant::now()`) and its wakeup alarm (the TIMG1 timer
//! passed to `esp_rtos::start_with_idle_hook`) — confirmed on hardware, both
//! lose the full nap. Neither esp-hal nor esp-rtos compensates for that, so
//! after every nap this module measures the slept time on the RTC slow clock
//! (which keeps running) and advances both counters by hand, like ESP-IDF does.
//!
//! esp-rtos arms TIMG1 as a one-shot that counts up from 0 to the next task
//! deadline, so `alarm - counter` is the time until the scheduler next needs
//! the CPU. Each nap is sized to end just before that deadline, and when the
//! deadline is too close to be worth sleeping the hook falls back to plain
//! `waiti` and lets the alarm fire normally. See Power-Management.md in the
//! Obsidian vault.
//!
//! ESP32-S3 has no USB-Serial-JTAG wake source, and every nap drops the USB
//! PHY, so a sleeping board is unreachable by probe-rs/espflash. Escape hatches:
//! - No sleep for `BOOT_GRACE` after reset — press reset, then attach.
//! - Press the provisioning button (GPIO2, an `Ext0WakeupSource`) to keep the
//!   CPU out of sleep for `BUTTON_AWAKE_WINDOW`.
//! - Strap GPIO0 to GND at reset to force ROM download mode — boot-ROM level,
//!   works regardless of firmware state, the backstop of last resort.

use core::sync::atomic::AtomicBool;

/// Shortest idle gap the hook will light-sleep through (shorter ones `waiti`).
/// Sweeping 5 down to 2.5 ms made no measurable difference (2026-10-17).
const MIN_NAP_US: u32 = 5_000;

/// Sleep-floor experiments (Sleep-Floor-Plan.md step 1), build env vars,
/// default 0 (off).
///
/// `SLEEP_VDDSDIO_PD=1`: power down the flash supply (VDD_SDIO) in light sleep.
/// ESP-IDF waits ~60µs for the flash after such a wake, from IRAM; this code
/// resumes in flash-resident code with no such wait, so it may crash on wake
/// (check the reset cause in telemetry).
pub const SLEEP_VDDSDIO_PD: bool = parse_u32(option_env!("SLEEP_VDDSDIO_PD"), 0) != 0;
/// `SLEEP_RTC_PERI_PD=1`: power down the RTC peripheral domain in light sleep.
/// `Ext0WakeupSource` needs that domain, so the GPIO2 wake button becomes a
/// digital GPIO wake instead. Watch the battery reading in telemetry: the
/// SAR ADC and RTC IO mux live in this domain.
pub const SLEEP_RTC_PERI_PD: bool = parse_u32(option_env!("SLEEP_RTC_PERI_PD"), 0) != 0;

/// `option_env!` value as a number, or `default` if unset (build-time knobs
/// such as `FETCH_GAP_S`). Fails the build on
/// anything but ASCII digits.
pub const fn parse_u32(s: Option<&str>, default: u32) -> u32 {
    let Some(s) = s else { return default };
    let b = s.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "expected a number");
        n = n * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    n
}

/// Set while the WiFi radio is started. The idle hook never light-sleeps while
/// this is set. Starts `true` because the radio comes up during boot (scan in
/// `wifi_connect::setup`) before the fetch loop takes over the flag.
pub static RADIO_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Set while the idle hook is in a light-sleep stretch (USB down, no RTT
/// reader). Always `false` without the `light-sleep` feature.
pub static SLEEPING: AtomicBool = AtomicBool::new(false);

/// Light-sleep totals since boot, for telemetry. All zero without the
/// `light-sleep` feature.
#[derive(Clone, Copy, Default)]
pub struct SleepTotals {
    pub slept_ms: u64,
    pub naps: u32,
    pub near_deadline_waits: u32,
    pub spurious_ext0: u32,
    /// Latest RC_SLOW calibration (real µs per million nominal); 0 if none.
    pub rc_scale_ppm: u32,
    /// `sleep-trace` only: embassy executor wakes while asleep, and task polls
    /// by spawn order (0 = `main`, then the `spawner.spawn` calls in `main.rs`).
    pub executor_wakes: u32,
    pub task_polls: [u32; 10],
}

static SLEEP_TOTALS: critical_section::Mutex<core::cell::Cell<SleepTotals>> =
    critical_section::Mutex::new(core::cell::Cell::new(SleepTotals {
        slept_ms: 0,
        naps: 0,
        near_deadline_waits: 0,
        spurious_ext0: 0,
        rc_scale_ppm: 0,
        executor_wakes: 0,
        task_polls: [0; 10],
    }));

/// Totals up to the end of the last sleeping stretch.
pub fn sleep_totals() -> SleepTotals {
    critical_section::with(|cs| SLEEP_TOTALS.borrow(cs).get())
}

#[cfg(feature = "light-sleep")]
mod inner {
    use core::cell::UnsafeCell;
    use core::sync::atomic::Ordering;
    use core::time::Duration;

    use esp_hal::clock::Clocks;
    use esp_hal::gpio::{Input, InputConfig, Pull, RtcPinWithResistors, WakeEvent};
    use esp_hal::peripherals::{GPIO2, LPWR, RTC_IO, SYSTIMER, TIMG1};
    use esp_hal::rtc_cntl::Rtc;
    use esp_hal::rtc_cntl::sleep::{
        Ext0WakeupSource, GpioWakeupSource, RtcSleepConfig, TimerWakeupSource, WakeupLevel,
    };
    use esp_hal::time::Instant;

    use super::{RADIO_ACTIVE, SLEEP_RTC_PERI_PD, SLEEP_VDDSDIO_PD};

    /// Longest single nap, used when no task deadline is armed.
    const MAX_NAP_US: u64 = 1_000_000;
    // Minimum nap: see `super::MIN_NAP_US`.
    /// When a nap reaches the deadline (every timer wake: naps run right up to
    /// it, so tasks run up to the wake-up latency late), leave TIMG1 this far
    /// short of its alarm so the next idle pass `waiti`s and the alarm fires by
    /// itself.
    const OVERSLEEP_LEAD_US: u64 = 20;
    /// No sleep for this long after reset, so probe-rs can always attach.
    const BOOT_GRACE: Duration = Duration::from_secs(20);
    /// No sleep for this long after a GPIO2 press wakes the CPU.
    const BUTTON_AWAKE_WINDOW: Duration = Duration::from_secs(30);

    /// `SLP_WAKEUP_CAUSE` bits (esp-hal's `WakeupReason`).
    const WAKEUP_CAUSE_EXT0: u32 = 1 << 0;
    const WAKEUP_CAUSE_GPIO: u32 = 1 << 2;
    const WAKEUP_CAUSE_TIMER: u32 = 1 << 3;
    /// Either wake button cause (ext0, or GPIO with `SLEEP_RTC_PERI_PD`).
    const WAKEUP_CAUSE_BUTTON: u32 = WAKEUP_CAUSE_EXT0 | WAKEUP_CAUSE_GPIO;
    /// SYSTIMER counts at a fixed 16 MHz on the S3 (XTAL / 2.5).
    const SYSTIMER_TICKS_PER_US: u64 = 16;
    /// RTC GPIO number of the GPIO2 wake button (same as the GPIO number on S3).
    const WAKE_BUTTON_RTC_GPIO: u32 = 2;

    /// RC_SLOW calibration: sample RTC vs SYSTIMER at most this often while
    /// awake, and only trust a ratio measured over at least `CAL_MIN_SPAN_US`.
    const CAL_SAMPLE_EVERY_US: u64 = 500_000;
    const CAL_MIN_SPAN_US: u64 = 2_000_000;
    /// Reject calibrations further than ±20% from nominal as bad reads.
    const CAL_MIN_PPM: u64 = 800_000;
    const CAL_MAX_PPM: u64 = 1_200_000;

    struct State {
        rtc: Rtc<'static>,
        sleep_cfg: RtcSleepConfig,
        wake_button: WakeButton,
        /// No sleep until `Instant::now()` passes this (boot grace / button window).
        awake_until_us: u64,
        /// Whether the CPU is in a sleeping stretch, to log only transitions.
        sleeping: bool,
        /// Last TIMG1 alarm and counter values seen, to count each newly armed
        /// timer once.
        last_alarm: u64,
        last_count: u64,
        /// Real µs per million esp-hal-nominal RTC µs. esp-hal assumes RC_SLOW
        /// runs at exactly 136 kHz and never calibrates it; on hardware it ran
        /// ~4-5% fast, so naps came up short and firmware time ran fast.
        rtc_scale_ppm: u64,
        /// (RTC µs, SYSTIMER ticks) at the start of the current awake stretch,
        /// and SYSTIMER ticks at the last sample.
        cal_anchor: Option<(u64, u64)>,
        cal_last_sys: u64,
        stats: SleepStats,
    }

    /// The GPIO2 escape-hatch wake: RTC ext0, or a digital GPIO wake when the
    /// RTC peripheral domain is powered down (`SLEEP_RTC_PERI_PD`).
    enum WakeButton {
        Ext0(Ext0WakeupSource<GPIO2<'static>>),
        Gpio(Input<'static>, GpioWakeupSource),
    }

    /// Accumulated over one sleeping stretch, printed (and reset) when it ends.
    /// Logs written while asleep stay in the RTT buffer until a GPIO2 press
    /// opens a window for probe-rs to read them.
    #[derive(Default)]
    struct SleepStats {
        naps: u32,
        /// Idle passes that `waiti`ed because a deadline was too close.
        near_deadline: u32,
        timer_wakes: u32,
        /// OR of every `SLP_WAKEUP_CAUSE` seen.
        wake_causes: u32,
        requested_us: u64,
        /// Time credited to SYSTIMER and TIMG1.
        slept_us: u64,
        /// RTC-measured bracket time, for diagnostics only (reads ~0, see
        /// `sleep_compensated`).
        rtc_us: u64,
        max_nap_us: u64,
        /// Naps that reached the TIMG1 alarm (every timer wake, now that naps run
        /// right up to the deadline).
        overslept: u32,
        /// ext0 wakes with GPIO2 not actually low — ignored.
        spurious_ext0: u32,
        /// Newly armed scheduler timers, bucketed by length (`ARMED_BUCKETS_MS`).
        armed: [u32; ARMED_BUCKETS_MS.len() + 1],
    }

    /// Upper bounds (ms) of the `SleepStats::armed` histogram buckets; the last
    /// bucket holds everything longer. A dominant bucket gives the period of
    /// whatever keeps waking the CPU.
    const ARMED_BUCKETS_MS: [u64; 7] = [2, 10, 20, 50, 100, 300, 1000];

    // `UnsafeCell` gives interior mutability without `static mut`.
    // `Sync` is implemented manually; soundness is guaranteed because every
    // access is wrapped in `critical_section::with`, which disables interrupts
    // and prevents the embassy executor from switching tasks — same idiom as
    // `logger.rs`.
    struct StateCell(UnsafeCell<Option<State>>);
    unsafe impl Sync for StateCell {}

    static STATE: StateCell = StateCell(UnsafeCell::new(None));

    /// Installs the RTC + wake-button handles. Call once from `main`, before
    /// `esp_rtos::start_with_idle_hook`.
    pub fn init(rtc: Rtc<'static>, wake_button: GPIO2<'static>) {
        // `Ext0WakeupSource` switches the pad to the RTC mux without enabling
        // the RTC-domain pull-up (the boot-time `Pull::Up` doesn't carry over),
        // so the pin floats and can read as pressed — seen on hardware as a
        // spurious ext0 wake on the very first nap.
        let wake_button = if SLEEP_RTC_PERI_PD {
            // The digital pad's pull-up stays on in light sleep.
            let mut input = Input::new(wake_button, InputConfig::default().with_pull(Pull::Up));
            input
                .wakeup_enable(true, WakeEvent::LowLevel)
                .expect("GPIO2 light-sleep wake");
            WakeButton::Gpio(input, GpioWakeupSource::new())
        } else {
            wake_button.rtcio_pullup(true);
            WakeButton::Ext0(Ext0WakeupSource::new(wake_button, WakeupLevel::Low))
        };
        critical_section::with(|_| {
            // SAFETY: interrupts disabled; no other accessor can exist.
            unsafe {
                *STATE.0.get() = Some(State {
                    rtc,
                    sleep_cfg: sleep_config(),
                    wake_button,
                    awake_until_us: BOOT_GRACE.as_micros() as u64,
                    sleeping: false,
                    last_alarm: u64::MAX,
                    last_count: 0,
                    rtc_scale_ppm: 1_000_000,
                    cal_anchor: None,
                    cal_last_sys: 0,
                    stats: SleepStats::default(),
                })
            };
        });
    }

    /// ESP-IDF's `rtc_sleep_get_default_config` light-sleep branch with XTAL
    /// and the internal 8 MHz oscillator powered down (esp32s3 `rtc.h`:
    /// `RTC_CNTL_DBIAS_SLP` = 5, `RTC_CNTL_DBG_ATTEN_LIGHTSLEEP_DEFAULT` = 5,
    /// bias/pd_cur DEFAULT = 1). On the PPK2 this took the asleep current from
    /// 11.9 mA (esp-hal's default config) to 4.4 mA; also powering down the
    /// modem domain saved only ~0.1 mA more (2026-10-17).
    fn sleep_config() -> RtcSleepConfig {
        let mut cfg = RtcSleepConfig::default();
        cfg.set_int_8m_pd_en(true);
        cfg.set_rtc_regulator_fpu(true);
        cfg.set_dbg_atten_slp(5);
        cfg.set_dig_dbias_slp(5);
        cfg.set_bias_sleep_slp(true);
        cfg.set_pd_cur_slp(true);
        cfg.set_bias_sleep_monitor(true);
        cfg.set_pd_cur_monitor(true);
        cfg.set_vddsdio_pd_en(SLEEP_VDDSDIO_PD);
        cfg.set_rtc_peri_pd_en(SLEEP_RTC_PERI_PD);
        cfg
    }

    fn now_us() -> u64 {
        Instant::now().duration_since_epoch().as_micros()
    }

    /// Custom esp-rtos idle hook: entered whenever the scheduler has no ready
    /// task. Its context is not preserved (see `esp_rtos::start_with_idle_hook`'s
    /// docs), so all state lives in `STATE` rather than locals carried across
    /// iterations. Falls back to the default hook's plain `waiti` whenever
    /// sleeping isn't allowed or isn't worth it.
    pub extern "C" fn idle_hook() -> ! {
        loop {
            let slept = critical_section::with(|_| {
                // SAFETY: interrupts disabled; no other accessor can exist.
                let Some(state) = (unsafe { (*STATE.0.get()).as_mut() }) else {
                    return false;
                };
                let radio_on = RADIO_ACTIVE.load(Ordering::Acquire);
                if radio_on || now_us() < state.awake_until_us {
                    if state.sleeping {
                        end_sleeping(state, if radio_on { "radio on" } else { "awake window" });
                    }
                    calibrate_rtc(state);
                    return false;
                }
                record_armed_timer(state);
                let Some(nap_us) = nap_budget_us() else {
                    state.stats.near_deadline += 1;
                    return false;
                };
                if !state.sleeping {
                    state.sleeping = true;
                    super::SLEEPING.store(true, Ordering::Relaxed);
                    // SYSTIMER is only advanced by estimate while asleep, so
                    // each awake stretch calibrates from scratch.
                    state.cal_anchor = None;
                    crate::dprintln!(
                        "light-sleep: entering at uptime {}ms, RC_SLOW scale {}ppm",
                        now_us() / 1000,
                        state.rtc_scale_ppm
                    );
                }
                let cause = sleep_compensated(state, nap_us);
                if cause & WAKEUP_CAUSE_BUTTON != 0 && !wake_button_low(state) {
                    state.stats.spurious_ext0 += 1;
                } else if cause & WAKEUP_CAUSE_BUTTON != 0 {
                    state.awake_until_us = now_us() + BUTTON_AWAKE_WINDOW.as_micros() as u64;
                    end_sleeping(state, "GPIO2 press");
                }
                true
            });
            if !slept {
                unsafe { core::arch::asm!("waiti 0") };
            }
        }
    }

    fn end_sleeping(state: &mut State, reason: &str) {
        state.sleeping = false;
        super::SLEEPING.store(false, Ordering::Relaxed);
        {
            let s = &state.stats;
            let rc_scale_ppm = state.rtc_scale_ppm as u32;
            critical_section::with(|cs| {
                let cell = super::SLEEP_TOTALS.borrow(cs);
                let mut t = cell.get();
                t.slept_ms += s.slept_us / 1000;
                t.naps += s.naps;
                t.near_deadline_waits += s.near_deadline;
                t.spurious_ext0 += s.spurious_ext0;
                t.rc_scale_ppm = rc_scale_ppm;
                #[cfg(feature = "sleep-trace")]
                {
                    let (polls, wakes) = super::trace::peek();
                    t.executor_wakes += wakes;
                    for (total, n) in t.task_polls.iter_mut().zip(polls) {
                        *total += n;
                    }
                }
                cell.set(t);
            });
        }
        #[cfg(feature = "sleep-trace")]
        {
            let (polls, wakes) = super::trace::take();
            crate::dprintln!(
                "light-sleep trace: {} executor wakes, task polls by spawn order: {:?}",
                wakes,
                polls
            );
        }
        let s = core::mem::take(&mut state.stats);
        let avg_nap_ms = s.slept_us / 1000 / s.naps.max(1) as u64;
        crate::dprintln!(
            "light-sleep: leaving ({}) at uptime {}ms after {} naps ({} near-deadline waits), \
             slept {}ms (avg nap {}ms, max {}ms; rtc-measured {}ms), \
             {} reached deadline, {} timer wakes, {} spurious ext0, causes {:#x}, \
             armed timers ms <=2/10/20/50/100/300/1000/more: {:?}",
            reason,
            now_us() / 1000,
            s.naps,
            s.near_deadline,
            s.slept_us / 1000,
            avg_nap_ms,
            s.max_nap_us / 1000,
            s.rtc_us / 1000,
            s.overslept,
            s.timer_wakes,
            s.spurious_ext0,
            s.wake_causes,
            s.armed
        );
    }

    /// Whether the GPIO2 wake button currently reads pressed (active low).
    fn wake_button_low(state: &State) -> bool {
        match &state.wake_button {
            WakeButton::Ext0(_) => {
                RTC_IO::regs().rtc_gpio_in().read().next().bits() & (1 << WAKE_BUTTON_RTC_GPIO) == 0
            }
            WakeButton::Gpio(input, _) => input.is_low(),
        }
    }

    /// Updates `rtc_scale_ppm` from how far the RTC slow clock and SYSTIMER
    /// have each moved since the start of this awake stretch. Called on idle
    /// passes that don't sleep, when SYSTIMER is running on the crystal and is
    /// the trusted reference. Rate-limited so it costs nothing measurable.
    fn calibrate_rtc(state: &mut State) {
        let sys = systimer_count();
        if sys.wrapping_sub(state.cal_last_sys) < CAL_SAMPLE_EVERY_US * SYSTIMER_TICKS_PER_US {
            return;
        }
        state.cal_last_sys = sys;
        latch_rtc_time();
        let rtc_us = state.rtc.time_since_boot().as_micros();
        let Some((rtc0, sys0)) = state.cal_anchor else {
            state.cal_anchor = Some((rtc_us, sys));
            return;
        };
        let real_us = (sys - sys0) / SYSTIMER_TICKS_PER_US;
        let nominal_us = rtc_us.saturating_sub(rtc0);
        if real_us < CAL_MIN_SPAN_US || nominal_us == 0 {
            return;
        }
        let ppm = real_us * 1_000_000 / nominal_us;
        if (CAL_MIN_PPM..=CAL_MAX_PPM).contains(&ppm) {
            state.rtc_scale_ppm = ppm;
        }
    }

    /// Counts a newly armed scheduler timer into the length histogram. esp-rtos
    /// resets TIMG1 to 0 whenever it arms a wakeup, so the alarm value is the
    /// full timeout it asked for, and a counter that went backwards marks a
    /// re-arm even when the timeout is the same as last time.
    fn record_armed_timer(state: &mut State) {
        if !TIMG1::regs().t(0).config().read().alarm_en().bit_is_set() {
            return;
        }
        let alarm = timg1_t0_alarm();
        let count = timg1_t0_count();
        let rearmed = alarm != state.last_alarm || count < state.last_count;
        state.last_alarm = alarm;
        state.last_count = count;
        if !rearmed {
            return;
        }
        let ms = alarm / timg1_t0_ticks_per_us() / 1000;
        let bucket = ARMED_BUCKETS_MS
            .iter()
            .position(|&max| ms <= max)
            .unwrap_or(ARMED_BUCKETS_MS.len());
        state.stats.armed[bucket] += 1;
    }

    /// How long the next nap may last, or `None` if the scheduler needs the CPU
    /// too soon (or its alarm already fired and the interrupt is pending).
    fn nap_budget_us() -> Option<u64> {
        let timg = TIMG1::regs();
        if timg.int_raw().read().t(0).bit_is_set() {
            return None;
        }
        let t = timg.t(0);
        let until_alarm_us = if t.config().read().alarm_en().bit_is_set() {
            timg1_t0_alarm().saturating_sub(timg1_t0_count()) / timg1_t0_ticks_per_us()
        } else {
            MAX_NAP_US
        };
        let budget = until_alarm_us.min(MAX_NAP_US);
        // No early-wake margin: a task running a little late is fine, and a
        // 2 ms margin cost a 2 ms awake wait on every wake.
        (budget >= super::MIN_NAP_US as u64).then_some(budget)
    }

    /// One light-sleep nap of up to `nap_us`, then advance SYSTIMER and TIMG1 by
    /// however long the CPU slept. Returns the wake cause bits. Must be called
    /// with interrupts disabled.
    ///
    /// On a timer-only wake the nap lasted `nap_us` — the wake alarm is set
    /// from a fresh RTC latch and scaled by the RC_SLOW calibration. Reading the RTC time straight after waking does *not* work: it
    /// comes back ~unchanged from before the nap (cause unknown), so it is only
    /// a fallback for GPIO2 wakes, whose length is otherwise unknown — at worst
    /// that loses up to one nap of firmware time per button press.
    fn sleep_compensated(state: &mut State, nap_us: u64) -> u32 {
        latch_rtc_time();
        let rtc_before = state.rtc.time_since_boot();
        let sys_before = systimer_count();

        // esp-hal converts this to RC_SLOW ticks at its nominal frequency, so
        // ask for the nominal duration that comes out to `nap_us` real time.
        let nominal_us = nap_us * 1_000_000 / state.rtc_scale_ppm;
        let timer = TimerWakeupSource::new(Duration::from_micros(nominal_us));
        // The wake alarm is set relative to an RTC read inside `sleep_light`.
        latch_rtc_time();
        match &state.wake_button {
            WakeButton::Ext0(ext0) => state.rtc.sleep(&state.sleep_cfg, &[&timer, ext0]),
            WakeButton::Gpio(_, gpio) => state.rtc.sleep(&state.sleep_cfg, &[&timer, gpio]),
        }

        let cause = LPWR::regs().slp_wakeup_cause().read().wakeup_cause().bits();
        latch_rtc_time();
        let rtc_us = (state.rtc.time_since_boot() - rtc_before).as_micros();
        let sys_us = (systimer_count() - sys_before) / SYSTIMER_TICKS_PER_US;

        let timer_only = cause & WAKEUP_CAUSE_TIMER != 0 && cause & WAKEUP_CAUSE_BUTTON == 0;
        // `nap_us` is time spent asleep, which neither counter saw at all; the
        // RTC fallback spans the whole bracket, so drop the part SYSTIMER saw.
        let slept_us = if timer_only {
            nap_us
        } else {
            rtc_us.saturating_sub(sys_us)
        };
        if slept_us > 0 {
            advance_systimer(slept_us);
            if advance_timg1_t0(slept_us) {
                state.stats.overslept += 1;
            }
        }

        let s = &mut state.stats;
        s.naps += 1;
        s.requested_us += nap_us;
        s.slept_us += slept_us;
        s.rtc_us += rtc_us;
        s.max_nap_us = s.max_nap_us.max(slept_us);
        s.wake_causes |= cause;
        if cause & WAKEUP_CAUSE_TIMER != 0 {
            s.timer_wakes += 1;
        }
        cause
    }

    /// Latches the current RTC time into `TIME_LOW0/HIGH0` and waits until it
    /// lands. On the S3 the latch takes up to a slow-clock cycle after
    /// `TIME_UPDATE` and there's no valid bit, but esp-hal's
    /// `time_since_boot_raw` reads straight after triggering — so without this
    /// it returns the *previous* latch. Both `Rtc::time_since_boot` and
    /// `TimerWakeupSource` go through that read, so calling this first makes
    /// them see a value only a few µs old.
    fn latch_rtc_time() {
        let rtc_cntl = LPWR::regs();
        let stale = rtc_cntl.time_low0().read().bits();
        rtc_cntl.time_update().write(|w| w.time_update().set_bit());
        // The RTC slow clock ticks every ~7µs, so a fresh latch differs from
        // the stale one within a couple of cycles. Bounded in case it doesn't.
        for _ in 0..10_000 {
            if rtc_cntl.time_low0().read().bits() != stale {
                break;
            }
        }
    }

    /// Raw SYSTIMER unit 0 count (same read sequence as esp-hal's `read_count`).
    fn systimer_count() -> u64 {
        let systimer = SYSTIMER::regs();
        systimer.unit_op(0).write(|w| w.update().set_bit());
        while !systimer.unit_op(0).read().value_valid().bit_is_set() {}
        let value = systimer.unit_value(0);
        loop {
            let lo = value.lo().read().bits();
            let hi = value.hi().read().bits();
            if lo == value.lo().read().bits() {
                return ((hi as u64) << 32) | lo as u64;
            }
        }
    }

    /// Moves `esp_hal::time::Instant::now()` forward by `us`.
    fn advance_systimer(us: u64) {
        let value = systimer_count() + us * SYSTIMER_TICKS_PER_US;
        let systimer = SYSTIMER::regs();
        // Not esp-hal's `set_count`: it shifts the high word the wrong way
        // (`value << 32`).
        systimer
            .unitload(0)
            .hi()
            .write(|w| unsafe { w.load_hi().bits((value >> 32) as u32) });
        systimer
            .unitload(0)
            .lo()
            .write(|w| unsafe { w.load_lo().bits(value as u32) });
        systimer.unit_load(0).write(|w| w.load().set_bit());
    }

    // All TIMG1 timer 0 accesses must match the timer handed to
    // `esp_rtos::start_with_idle_hook` in `main.rs`.
    fn timg1_t0_count() -> u64 {
        let t = TIMG1::regs().t(0);
        t.update().write(|w| w.update().set_bit());
        while t.update().read().update().bit_is_set() {}
        ((t.hi().read().bits() as u64) << 32) | t.lo().read().bits() as u64
    }

    fn timg1_t0_alarm() -> u64 {
        let t = TIMG1::regs().t(0);
        ((t.alarmhi().read().bits() as u64) << 32) | t.alarmlo().read().bits() as u64
    }

    /// TIMG1 timer 0 ticks per microsecond (APB / divider), at least 1.
    fn timg1_t0_ticks_per_us() -> u64 {
        // Same decoding as esp-hal's `Timer::divider` (TRM: a DIVIDER of 1 or
        // 2 divides by 2, 0 by 65536). esp-rtos's TIMG1 runs with DIVIDER = 1,
        // and reading that as /1 doubled every tick count here: naps came out
        // half as long as the time to the alarm, then pushed TIMG1 twice as
        // far, so each timer took ~4 naps and as many early alarms
        // (2026-10-18; ~11.4 -> ~6.7 mA average).
        let divider = match TIMG1::regs().t(0).config().read().divider().bits() {
            0 => 65536,
            1 | 2 => 2,
            d => d as u64,
        };
        (Clocks::get().apb_clock.as_hz() as u64 / 1_000_000 / divider).max(1)
    }

    /// Moves esp-rtos's pending wakeup alarm forward by `us`, so it fires on the
    /// real-time schedule. If that would reach an armed alarm (the nap ran to
    /// its deadline), stops `OVERSLEEP_LEAD_US` short instead so the next idle
    /// pass `waiti`s and the comparator fires by itself — this doesn't depend
    /// on whether it triggers on an exact match or on passing. Returns whether
    /// that happened.
    fn advance_timg1_t0(us: u64) -> bool {
        let t = TIMG1::regs().t(0);
        let ticks_per_us = timg1_t0_ticks_per_us();
        let current = timg1_t0_count();
        let mut target = current + us * ticks_per_us;
        let mut overslept = false;

        if t.config().read().alarm_en().bit_is_set() {
            let alarm = timg1_t0_alarm();
            if target >= alarm {
                target = alarm
                    .saturating_sub(OVERSLEEP_LEAD_US * ticks_per_us)
                    .max(current);
                overslept = true;
            }
        }

        t.loadhi()
            .write(|w| unsafe { w.load_hi().bits((target >> 32) as u32) });
        t.loadlo()
            .write(|w| unsafe { w.load_lo().bits(target as u32) });
        t.load().write(|w| unsafe { w.load().bits(1) });
        overslept
    }
}

#[cfg(feature = "light-sleep")]
pub use inner::{idle_hook, init};

/// `embassy-executor` `trace` hooks: counts how often each embassy task is
/// polled while light-sleeping. Tasks are identified by spawn order (index 0 is
/// `main`, then the `spawner.spawn` calls in `main.rs`). Executor wakes well
/// below the idle hook's naps + near-deadline waits means something outside
/// embassy (e.g. esp-radio's timer thread) is waking the CPU.
#[cfg(feature = "sleep-trace")]
mod trace {
    use core::cell::RefCell;
    use core::sync::atomic::Ordering;

    use critical_section::Mutex;

    use super::SLEEPING;

    const MAX_TASKS: usize = 10;

    struct Trace {
        ids: [u32; MAX_TASKS],
        spawned: usize,
        polls: [u32; MAX_TASKS],
        wakes: u32,
    }

    static TRACE: Mutex<RefCell<Trace>> = Mutex::new(RefCell::new(Trace {
        ids: [0; MAX_TASKS],
        spawned: 0,
        polls: [0; MAX_TASKS],
        wakes: 0,
    }));

    /// Per-task poll counts and executor wakes since the last `take`, without
    /// resetting them.
    pub fn peek() -> ([u32; MAX_TASKS], u32) {
        critical_section::with(|cs| {
            let t = TRACE.borrow_ref(cs);
            (t.polls, t.wakes)
        })
    }

    /// Per-task poll counts and executor wakes since the last call.
    pub fn take() -> ([u32; MAX_TASKS], u32) {
        critical_section::with(|cs| {
            let mut t = TRACE.borrow_ref_mut(cs);
            let out = (t.polls, t.wakes);
            t.polls = [0; MAX_TASKS];
            t.wakes = 0;
            out
        })
    }

    #[unsafe(no_mangle)]
    fn _embassy_trace_task_new(_executor_id: u32, task_id: u32) {
        critical_section::with(|cs| {
            let mut t = TRACE.borrow_ref_mut(cs);
            if t.spawned < MAX_TASKS {
                let i = t.spawned;
                t.ids[i] = task_id;
                t.spawned += 1;
            }
        });
    }

    #[unsafe(no_mangle)]
    fn _embassy_trace_task_exec_begin(_executor_id: u32, task_id: u32) {
        if !SLEEPING.load(Ordering::Relaxed) {
            return;
        }
        critical_section::with(|cs| {
            let mut t = TRACE.borrow_ref_mut(cs);
            let spawned = t.spawned;
            if let Some(i) = t.ids[..spawned].iter().position(|&id| id == task_id) {
                t.polls[i] += 1;
            }
        });
    }

    #[unsafe(no_mangle)]
    fn _embassy_trace_poll_start(_executor_id: u32) {
        if !SLEEPING.load(Ordering::Relaxed) {
            return;
        }
        critical_section::with(|cs| TRACE.borrow_ref_mut(cs).wakes += 1);
    }

    #[unsafe(no_mangle)]
    fn _embassy_trace_task_end(_executor_id: u32, _task_id: u32) {}
    #[unsafe(no_mangle)]
    fn _embassy_trace_task_exec_end(_executor_id: u32, _task_id: u32) {}
    #[unsafe(no_mangle)]
    fn _embassy_trace_task_ready_begin(_executor_id: u32, _task_id: u32) {}
    #[unsafe(no_mangle)]
    fn _embassy_trace_executor_idle(_executor_id: u32) {}
}
