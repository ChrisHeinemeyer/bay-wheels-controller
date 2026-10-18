//! Per-fetch-cycle status report to a LAN log server (`log-server` feature).
//!
//! Sent from inside the fetch cycle while the radio is already up, so it adds
//! no wakeups and no radio-on time beyond the POST itself. Plain HTTP/1.1 over
//! the fetch task's own `TcpClient` (its single socket is free again once the
//! GBFS connection is dropped), straight to `LOG_SERVER` from `.env` — no DNS,
//! no reqwless, and no new big buffers (the main stack has <7KB headroom, see
//! Power-Management.md). Any failure is logged and ignored.
//!
//! The receiving end is the Home Assistant add-on in `homeassistant/`.

use core::fmt::Write as _;
use core::net::SocketAddr;

use embassy_net::tcp::client::TcpClient;
use embassy_time::{Duration, Instant, with_timeout};
use embedded_io_async::{Read as _, Write as _};
use embedded_nal_async::TcpConnect as _;

use crate::GIT_VERSION;
use crate::tasks::signals::STATUS;
use crate::watchdog;

const LOG_SERVER: &str = env!("LOG_SERVER");
/// Upper bound on the whole connect+send+response, so a dead server can't
/// hold the radio on.
const POST_TIMEOUT: Duration = Duration::from_secs(5);

/// What the fetch loop knows about the current and previous cycle.
pub struct CycleReport {
    pub fetch_ok: bool,
    pub fetch_ok_total: u32,
    pub fetch_fail_total: u32,
    /// Start of cycle → WiFi associated.
    pub connect_ms: u32,
    /// Associated → DHCP lease.
    pub dhcp_ms: u32,
    /// DNS + TLS + GBFS download/parse.
    pub fetch_ms: u32,
    /// Whole radio-on window of the previous cycle (this one's isn't over).
    pub prev_radio_on_ms: u32,
    pub heap: HeapSteps,
    pub timings: super::fetch::FetchTimings,
    /// Whether this cycle reused the previous DHCP lease (no DHCP exchange).
    pub lease_reused: bool,
}

/// Free heap (bytes) at each step of the WiFi cycle, to localize a leak.
#[derive(Default)]
pub struct HeapSteps {
    /// Previous cycle, after `teardown()` (radio stopped, nothing in flight).
    pub prev_after_teardown: u32,
    /// This cycle, before `start_async()`.
    pub start: u32,
    pub after_start: u32,
    pub after_connect: u32,
    pub after_dhcp: u32,
    /// After DNS + TLS + download (connection dropped).
    pub after_fetch: u32,
}

pub async fn send_report<const N: usize, const TX: usize, const RX: usize>(
    tcp: &TcpClient<'static, N, TX, RX>,
    cycle: &CycleReport,
) {
    let mut body = heapless::String::<1536>::new();
    let has_reset_summary = build_body(&mut body, cycle).await;

    match with_timeout(POST_TIMEOUT, post(tcp, &body)).await {
        Ok(Ok(())) => {
            crate::dprintln!("✓ Telemetry sent ({} bytes)", body.len());
            if has_reset_summary {
                watchdog::mark_reset_summary_delivered();
            }
        }
        Ok(Err(e)) => crate::dprintln!("✗ Telemetry failed: {}", e),
        Err(_) => crate::dprintln!("✗ Telemetry timed out"),
    }
}

async fn post<const N: usize, const TX: usize, const RX: usize>(
    tcp: &TcpClient<'static, N, TX, RX>,
    body: &str,
) -> Result<(), &'static str> {
    let addr: SocketAddr = LOG_SERVER.parse().map_err(|_| "bad LOG_SERVER")?;
    let mut conn = tcp.connect(addr).await.map_err(|_| "connect")?;

    let mut head = heapless::String::<160>::new();
    write!(
        head,
        "POST /report HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        LOG_SERVER,
        body.len()
    )
    .map_err(|_| "header")?;
    conn.write_all(head.as_bytes()).await.map_err(|_| "write")?;
    conn.write_all(body.as_bytes()).await.map_err(|_| "write")?;
    conn.flush().await.map_err(|_| "flush")?;

    // Only the status line matters: "HTTP/1.1 2xx".
    let mut resp = [0u8; 16];
    let mut got = 0;
    while got < 12 {
        match conn.read(&mut resp[got..]).await {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return Err("read"),
        }
    }
    if resp.get(9) == Some(&b'2') {
        Ok(())
    } else {
        Err("non-2xx response")
    }
}

/// Fills `out` with the JSON report. Returns whether it carries the pending
/// reset summary (so it can be marked delivered on success).
async fn build_body(out: &mut heapless::String<1536>, cycle: &CycleReport) -> bool {
    let (board, battery_pct, battery_adc_mv, rssi, tx_power, last_fetch_at) = {
        let s = STATUS.lock().await;
        (
            s.board_id,
            s.battery_pct,
            s.battery_adc_mv,
            s.rssi,
            s.tx_power,
            s.last_fetch_at,
        )
    };
    let now = Instant::now();
    let sleep = crate::power::sleep_totals();

    let _ = write!(
        out,
        "{{\"board\":\"{:?}\",\"fw\":\"{}\",\"boot\":{},\"uptime_s\":{},\
         \"battery_pct\":{},\"battery_adc_mv\":{},\"rssi\":{},\"tx_power_qdbm\":{},\
         \"fetch_ok\":{},\"fetch_ok_total\":{},\"fetch_fail_total\":{},",
        board,
        GIT_VERSION,
        watchdog::boot_count(),
        now.as_secs(),
        battery_pct,
        battery_adc_mv,
        rssi,
        tx_power,
        cycle.fetch_ok,
        cycle.fetch_ok_total,
        cycle.fetch_fail_total,
    );
    match last_fetch_at {
        Some(t) => {
            let _ = write!(out, "\"fetch_age_s\":{},", now.duration_since(t).as_secs());
        }
        None => {
            let _ = out.push_str("\"fetch_age_s\":null,");
        }
    }
    let _ = write!(
        out,
        "\"connect_ms\":{},\"dhcp_ms\":{},\"fetch_ms\":{},\"prev_radio_on_ms\":{},\
         \"slept_ms_total\":{},\"naps_total\":{},\"near_deadline_waits_total\":{},\
         \"spurious_ext0_total\":{},\"rc_scale_ppm\":{},\"heap_free\":{},\
         \"heap_prev_after_teardown\":{},\"heap_cycle_start\":{},\"heap_after_start\":{},\
         \"heap_after_connect\":{},\"heap_after_dhcp\":{},\"heap_after_fetch\":{},\
         \"sleep_cfg\":{},\"min_nap_us\":{},\"handshake_ms\":{},\"ttfb_ms\":{},\
         \"body_ms\":{},\"parse_ms\":{},\"body_bytes\":{},\"src\":\"{}\",\"lease\":\"{}\",\"lease_s\":{},\
         \"first_group_rx_ms\":{},\"first_unicast_rx_ms\":{}",
        cycle.connect_ms,
        cycle.dhcp_ms,
        cycle.fetch_ms,
        cycle.prev_radio_on_ms,
        sleep.slept_ms,
        sleep.naps,
        sleep.near_deadline_waits,
        sleep.spurious_ext0,
        sleep.rc_scale_ppm,
        esp_alloc::HEAP.free(),
        cycle.heap.prev_after_teardown,
        cycle.heap.start,
        cycle.heap.after_start,
        cycle.heap.after_connect,
        cycle.heap.after_dhcp,
        cycle.heap.after_fetch,
        crate::power::sleep_level(),
        crate::power::min_nap_us(),
        cycle.timings.handshake_ms,
        cycle.timings.ttfb_ms,
        cycle.timings.body_ms,
        cycle.timings.parse_ms,
        cycle.timings.body_bytes,
        if cycle.timings.from_mirror {
            "mirror"
        } else {
            "lyft"
        },
        if cycle.lease_reused { "reused" } else { "dhcp" },
        smoltcp::socket::dhcpv4::LAST_LEASE_SECS.load(core::sync::atomic::Ordering::Relaxed),
        cycle.timings.first_group_rx_ms.map_or(-1, |v| v as i64),
        cycle.timings.first_unicast_rx_ms.map_or(-1, |v| v as i64),
    );

    let mut summary = heapless::String::<256>::new();
    let has_reset_summary = watchdog::pending_reset_summary(&mut summary);
    if has_reset_summary {
        let _ = out.push_str(",\"reset\":\"");
        push_json_escaped(out, &summary);
        let _ = out.push('"');
        let mut bt = heapless::Vec::<u32, { watchdog::BACKTRACE_WORDS }>::new();
        watchdog::reset_backtrace(&mut bt);
        if !bt.is_empty() {
            let _ = out.push_str(",\"backtrace\":[");
            for (i, pc) in bt.iter().enumerate() {
                let _ = write!(out, "{}\"{:#010x}\"", if i == 0 { "" } else { "," }, pc);
            }
            let _ = out.push(']');
        }
    }
    let _ = out.push('}');
    has_reset_summary
}

fn push_json_escaped<const N: usize>(out: &mut heapless::String<N>, s: &str) {
    for c in s.chars() {
        let _ = match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        };
    }
}
