//! Self-recovery: the RTC watchdog, stall detection, and a reset record that
//! survives the reset so the next boot can say why it restarted.
//!
//! Without this, any panic (including esp-hal's stack-guard and unhandled
//! exception panics) or hang leaves the board dead until power-cycled — the
//! panic handler used to be `loop {}` and no watchdog was enabled.
//!
//! The RTC watchdog (RWDT) runs on the RTC slow clock, so unlike the TIMG
//! watchdogs it keeps counting through CPU light sleep (see `power.rs`).

use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use esp_hal::rtc_cntl::{Rtc, RwdtStage, SocResetReason};

/// Reset if the watchdog isn't fed for this long. Fed every `FEED_EVERY_SECS`.
const RWDT_TIMEOUT_SECS: u64 = 60;
pub const FEED_EVERY_SECS: u64 = 10;
/// Reset if there has been no successful fetch for this long (since boot, if
/// none yet). Long enough to ride out a router reboot without reset-looping.
const FETCH_STALL_SECS: u64 = 15 * 60;

/// Bumped by the input task on every poll; checked by `check_and_feed`.
pub static INPUT_HEARTBEAT: AtomicU32 = AtomicU32::new(0);

/// Why the firmware reset itself, as stored in the reset record.
#[derive(Clone, Copy, Debug)]
pub enum ResetCause {
    Panic = 1,
    FetchStall = 2,
    InputStall = 3,
}

const RECORD_MAGIC: u32 = 0xB1CE_5EED;
const FILE_WORDS: usize = 12; // 48 bytes
const MSG_WORDS: usize = 16; // 64 bytes
const RECORD_WORDS: usize = 4 + FILE_WORDS + MSG_WORDS;

/// `[magic, cause, line, uptime_s, file.., message..]`, as words so it is
/// `AnyBitPattern` without extra derives. RTC fast RAM survives
/// `software_reset()` and watchdog resets (not power loss).
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut RESET_RECORD: [u32; RECORD_WORDS] = [0; RECORD_WORDS];

static PANICKING: AtomicBool = AtomicBool::new(false);

/// Boots since power-on (survives software/watchdog resets, not power loss),
/// stored as `[count, !count]`. The check word matters: after flashing the
/// first reset is a software reset, not power-on, so the RTC RAM still holds
/// garbage — seen on hardware as boot #1049984084.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut BOOT_COUNT: [u32; 2] = [0; 2];

/// One-line account of why the previous boot ended, built at boot for
/// telemetry, and whether it still needs to be delivered.
static RESET_SUMMARY: critical_section::Mutex<core::cell::RefCell<heapless::String<256>>> =
    critical_section::Mutex::new(core::cell::RefCell::new(heapless::String::new()));
static RESET_SUMMARY_PENDING: AtomicBool = AtomicBool::new(false);

pub fn boot_count() -> u32 {
    // SAFETY: plain word read; only written once at boot.
    unsafe {
        core::ptr::addr_of!(BOOT_COUNT)
            .cast::<u32>()
            .read_volatile()
    }
}

/// Copies the reset summary into `out` if it hasn't been delivered yet.
pub fn pending_reset_summary(out: &mut heapless::String<256>) -> bool {
    if !RESET_SUMMARY_PENDING.load(Ordering::Relaxed) {
        return false;
    }
    critical_section::with(|cs| {
        out.clear();
        let _ = out.push_str(&RESET_SUMMARY.borrow_ref(cs));
    });
    true
}

/// Call once the summary has reached the log server.
pub fn mark_reset_summary_delivered() {
    RESET_SUMMARY_PENDING.store(false, Ordering::Relaxed);
}

fn set_reset_summary(args: core::fmt::Arguments) {
    critical_section::with(|cs| {
        let mut summary = RESET_SUMMARY.borrow_ref_mut(cs);
        summary.clear();
        let _ = summary.write_fmt(args);
    });
    RESET_SUMMARY_PENDING.store(true, Ordering::Relaxed);
}

struct Watchdog {
    rtc: Rtc<'static>,
}

// Only touched from `main` (init) and the main task's keep-alive loop.
static mut WATCHDOG: Option<Watchdog> = None;

/// Starts the RTC watchdog. `rtc` only needs to provide `rwdt`; the light-sleep
/// code owns a separate `Rtc` for the sleep/time registers.
pub fn init(mut rtc: Rtc<'static>) {
    rtc.rwdt.set_timeout(
        RwdtStage::Stage0,
        esp_hal::time::Duration::from_secs(RWDT_TIMEOUT_SECS),
    );
    rtc.rwdt.enable();
    rtc.rwdt.feed();
    // SAFETY: called once from `main` before the keep-alive loop starts; no
    // other accessor exists.
    unsafe { *core::ptr::addr_of_mut!(WATCHDOG) = Some(Watchdog { rtc }) };
}

/// Called every `FEED_EVERY_SECS` from `main`'s keep-alive loop. Resets (with a
/// record) if the fetch loop or input task has stalled, otherwise feeds the
/// RWDT. If this loop itself stops running — a hang anywhere that blocks the
/// executor — the RWDT resets the chip.
pub fn check_and_feed(last_fetch_ok_us: u64, last_input_beat: &mut u32) {
    let now_us = embassy_time::Instant::now().as_micros();
    if now_us.saturating_sub(last_fetch_ok_us) > FETCH_STALL_SECS * 1_000_000 {
        record_and_reset(
            ResetCause::FetchStall,
            file!(),
            line!(),
            format_args!(
                "no successful fetch for {}s",
                now_us.saturating_sub(last_fetch_ok_us) / 1_000_000
            ),
        );
    }
    let beat = INPUT_HEARTBEAT.load(Ordering::Relaxed);
    if beat == *last_input_beat {
        record_and_reset(
            ResetCause::InputStall,
            file!(),
            line!(),
            format_args!("input task stopped polling for {}s", FEED_EVERY_SECS),
        );
    }
    *last_input_beat = beat;

    // SAFETY: only the keep-alive loop calls this, after `init`.
    if let Some(wd) = unsafe { (*core::ptr::addr_of_mut!(WATCHDOG)).as_mut() } {
        wd.rtc.rwdt.feed();
    }
}

/// Writes the reset record and resets. Safe to call from the panic handler;
/// a panic while already panicking resets immediately.
pub fn record_and_reset(
    cause: ResetCause,
    file: &str,
    line: u32,
    message: core::fmt::Arguments,
) -> ! {
    if PANICKING.swap(true, Ordering::Relaxed) {
        esp_hal::system::software_reset();
    }
    let rec = core::ptr::addr_of_mut!(RESET_RECORD).cast::<u32>();
    let uptime_s = embassy_time::Instant::now().as_secs() as u32;
    // SAFETY: plain word writes into a static nothing else is writing; the
    // magic goes last so a reset mid-write leaves an invalid record.
    unsafe {
        rec.add(1).write_volatile(cause as u32);
        rec.add(2).write_volatile(line);
        rec.add(3).write_volatile(uptime_s);
        // Keep the tail of the path — the file name is the useful part.
        let tail = &file.as_bytes()[file.len().saturating_sub(FILE_WORDS * 4)..];
        write_bytes(rec.add(4), FILE_WORDS, tail);
        let mut msg = ByteBuf::<{ MSG_WORDS * 4 }>::new();
        let _ = msg.write_fmt(message);
        write_bytes(rec.add(4 + FILE_WORDS), MSG_WORDS, msg.as_bytes());
        rec.write_volatile(RECORD_MAGIC);
    }
    esp_hal::system::software_reset()
}

/// Logs why the previous boot ended — the hardware reset reason, plus the
/// firmware's own record if it reset itself — then clears the record.
pub fn report_last_reset() {
    let reason = esp_hal::system::reset_reason();
    // SAFETY: single-threaded early boot; plain word reads/writes.
    let boot = unsafe {
        let count = core::ptr::addr_of_mut!(BOOT_COUNT).cast::<u32>();
        let (n, check) = (count.read_volatile(), count.add(1).read_volatile());
        let valid = check == !n && reason != Some(SocResetReason::ChipPowerOn);
        let next = if valid { n.wrapping_add(1) } else { 1 };
        count.write_volatile(next);
        count.add(1).write_volatile(!next);
        next
    };
    let (name, meaning) = describe(reason);
    crate::dprintln!(
        "Reset reason: {} - {} (boot #{} since power-on)",
        name,
        meaning,
        boot
    );

    let rec = core::ptr::addr_of_mut!(RESET_RECORD).cast::<u32>();
    // SAFETY: single-threaded early boot; plain word reads/writes.
    unsafe {
        if rec.read_volatile() != RECORD_MAGIC {
            let note = match reason {
                Some(SocResetReason::CoreSw | SocResetReason::CpuSw) => {
                    " No crash record, so not a firmware panic/stall reset - most likely \
                     flashing or a debugger/esptool reset."
                }
                _ => "",
            };
            set_reset_summary(format_args!("{}: {}.{}", name, meaning, note));
            return;
        }
        let cause = match rec.add(1).read_volatile() {
            1 => "panic",
            2 => "fetch stall",
            3 => "input stall",
            _ => "unknown",
        };
        let mut file = [0u8; FILE_WORDS * 4];
        let mut msg = [0u8; MSG_WORDS * 4];
        read_bytes(rec.add(4), &mut file);
        read_bytes(rec.add(4 + FILE_WORDS), &mut msg);
        let (line, uptime_s) = (rec.add(2).read_volatile(), rec.add(3).read_volatile());
        crate::dprintln!(
            "Previous boot reset itself: {} at {}:{} after {}s uptime: {}",
            cause,
            as_str(&file),
            line,
            uptime_s,
            as_str(&msg)
        );
        set_reset_summary(format_args!(
            "Firmware reset itself ({}) after {}s uptime at {}:{}: {}",
            cause,
            uptime_s,
            as_str(&file),
            line,
            as_str(&msg)
        ));
        rec.write_volatile(0);
    }
}

/// Short name and plain-English meaning of a hardware reset reason, in terms
/// of what it implies for this firmware.
fn describe(reason: Option<SocResetReason>) -> (&'static str, &'static str) {
    use SocResetReason::*;
    match reason {
        None => ("unknown", "reset reason register unreadable"),
        Some(ChipPowerOn) => (
            "power-on",
            "power was applied or restored (batteries or USB), or a brownout deep \
             enough to look like power loss",
        ),
        Some(CoreSw) => (
            "software reset",
            "software_reset(): flashing, or this firmware",
        ),
        Some(CpuSw) => ("CPU software reset", "CPU-only software reset (debugger)"),
        Some(CoreDeepSleep) => ("deep-sleep wake", "woke from deep sleep"),
        Some(CoreMwdt0 | CoreMwdt1 | CpuMwdt0 | CpuMwdt1) => (
            "timer-group watchdog",
            "a TIMG watchdog fired (this firmware doesn't arm those, so unexpected)",
        ),
        Some(CoreRtcWdt | CpuRtcWdt | SysRtcWdt) => (
            "RTC watchdog",
            "firmware hung: the keep-alive loop stopped feeding the watchdog for 60s",
        ),
        Some(SysBrownOut) => (
            "brownout",
            "supply voltage sagged too low (weak batteries, likely during a WiFi TX burst)",
        ),
        Some(SysSuperWdt) => ("super watchdog", "hardware super-watchdog fired"),
        Some(SysClkGlitch) => ("clock glitch", "clock glitch detected"),
        Some(CorePwrGlitch) => ("power glitch", "power glitch detected"),
        Some(CoreEfuseCrc) => ("eFuse CRC error", "eFuse CRC check failed"),
        Some(CoreUsbUart) => ("USB-UART reset", "reset requested over USB serial"),
        Some(CoreUsbJtag) => (
            "USB-JTAG reset",
            "reset requested over USB-JTAG (probe-rs/espflash)",
        ),
    }
}

unsafe fn write_bytes(dst: *mut u32, words: usize, bytes: &[u8]) {
    for w in 0..words {
        let mut word = [0u8; 4];
        for (i, b) in word.iter_mut().enumerate() {
            *b = bytes.get(w * 4 + i).copied().unwrap_or(0);
        }
        unsafe { dst.add(w).write_volatile(u32::from_le_bytes(word)) };
    }
}

unsafe fn read_bytes(src: *const u32, out: &mut [u8]) {
    for (w, chunk) in out.chunks_mut(4).enumerate() {
        let word = unsafe { src.add(w).read_volatile() }.to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
}

fn as_str(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    // Truncation can split a UTF-8 sequence; fall back to the valid prefix.
    match core::str::from_utf8(&bytes[..end]) {
        Ok(s) => s,
        Err(e) => core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    }
}

/// Fixed-size `fmt::Write` sink that silently truncates.
struct ByteBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> ByteBuf<N> {
    fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> core::fmt::Write for ByteBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
