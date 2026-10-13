use super::station_parser::StreamingStationParser;
use super::wifi_connect;
use core::sync::atomic::Ordering;
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use esp_radio::wifi::{PowerSaveMode, WifiController};
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::RequestBuilder,
};
use static_cell::StaticCell;

use crate::{
    power::RADIO_ACTIVE,
    stations::{STATION_DATA_LEN, TARGET_STATIONS},
    tasks::{
        signals::{FETCH_SIGNAL, STATION_DATA_SIGNAL, STATUS},
        station_parser::StationData,
    },
};

const HOST_URL: &str = "https://gbfs.lyft.com";
const PATH: &str = "/gbfs/2.3/bay/en/station_status.json";

/// How long to wait for DHCP after a reconnect before giving up on this cycle.
const DHCP_TIMEOUT: Duration = Duration::from_secs(20);
/// TCP transmit buffer for the fetch task's single socket.
const TCP_TX_SIZE: usize = 4096;
/// Upper bound on DNS + TLS + download + parse. The body read has no timeout of
/// its own, so a server that stalls mid-body would otherwise hold the radio on
/// until the watchdog's fetch-stall reset.
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Gap after any failed cycle (start/connect/DHCP/fetch) before retrying.
const RETRY_GAP: Duration = Duration::from_secs(60);
/// Gap after a successful fetch before the next cycle — the radio is stopped for
/// all of it, so this is where CPU light sleep (src/power.rs) takes effect.
const SUCCESS_GAP: Duration = Duration::from_secs(120);

/// Outcome of one fetch attempt over a freshly-opened connection.
enum FetchOutcome {
    /// Body streamed and parsed successfully.
    Success,
    /// The connection is now in an unknown state (write/read error) — reconnect from scratch.
    ConnectionLost,
}

/// Drives the WiFi controller through a full connect → fetch → teardown cycle
/// every `SUCCESS_GAP` (or `RETRY_GAP` after a failure), rather than staying
/// associated with modem-sleep between fetches — see Power-Management.md for why:
/// this Rust WiFi/RTOS stack has no automatic-light-sleep/PM-lock coordination, so
/// a future CPU light-sleep idle hook is only safe once the radio is fully
/// stopped, not just modem-sleeping.
#[embassy_executor::task]
pub async fn fetch_task(
    stack: &'static Stack<'static>,
    mut controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
    // One-time boot scan + config; every later connect() reuses it.
    wifi_connect::setup(&mut controller, ssid, password).await;

    // Create static buffers for TCP client state and TLS.
    // TLS needs large buffers for handshake AND streaming - increase for large responses
    // TcpClientState<sockets, TX, RX>. TX only ever carries the GET, the TLS
    // handshake and the telemetry POST (well under 4KB); RX streams the ~275KB
    // body. TX was 32KB — the 28KB saved goes to the main stack, which is
    // whatever DRAM is left after statics and had <7KB headroom in the TLS
    // handshake (see Power-Management.md).
    static TCP_CLIENT_STATE: StaticCell<TcpClientState<1, TCP_TX_SIZE, 32768>> = StaticCell::new();
    static TLS_RX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();
    static TLS_TX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();

    let tcp_state = TCP_CLIENT_STATE.init(TcpClientState::new());
    let tls_read_buffer = TLS_RX_BUFFER.init([0; 32768]);
    let tls_write_buffer = TLS_TX_BUFFER.init([0; 32768]);

    let tcp_client = TcpClient::new(*stack, tcp_state);
    let dns_socket = DnsSocket::new(*stack);

    // Adaptive TX power, now re-based on successful connect+fetch cycles instead
    // of 5-second maintenance ticks (there's no multi-minute "stable connection"
    // clock left once the radio is torn down every cycle). See wifi_connect.rs.
    let mut tx_power = wifi_connect::TX_POWER_MAX;
    let mut success_streak: u32 = 0;
    #[cfg(feature = "log-server")]
    let (mut fetch_ok_total, mut fetch_fail_total, mut prev_radio_on_ms) = (0u32, 0u32, 0u32);
    // Free heap after the previous cycle's teardown (radio stopped, nothing
    // in flight) — with `heap.start` below, the clean per-cycle leak signal.
    #[cfg(feature = "log-server")]
    let mut prev_heap_after_teardown = 0u32;
    #[cfg(feature = "log-server")]
    let heap = || esp_alloc::HEAP.free() as u32;

    loop {
        let cycle_start = Instant::now();
        #[cfg(feature = "log-server")]
        let mut heap_steps = super::telemetry::HeapSteps {
            start: heap(),
            prev_after_teardown: prev_heap_after_teardown,
            ..Default::default()
        };
        // Hold off CPU light-sleep (src/power.rs) from here until teardown()
        // confirms the radio is stopped.
        RADIO_ACTIVE.store(true, Ordering::Release);

        // `setup()` already started the controller once (it needs the radio up to
        // scan) — only call `start_async()` on cycles where `teardown()` actually
        // stopped it. Calling it on an already-started controller hangs forever:
        // `start_async()` awaits a fresh `StaStart` event that never fires again
        // since nothing transitions state the second time.
        if !controller.is_started().unwrap_or(false) {
            if let Err(e) = controller.start_async().await {
                crate::dprintln!("✗ WiFi start failed: {:?}", e);
                RADIO_ACTIVE.store(controller.is_started().unwrap_or(true), Ordering::Release);
                reset_tx_power(&mut tx_power, &mut success_streak);
                Timer::after(RETRY_GAP).await;
                continue;
            }
        }
        #[cfg(feature = "log-server")]
        {
            heap_steps.after_start = heap();
        }

        let connected_rssi = match wifi_connect::connect(&mut controller, tx_power).await {
            Ok(rssi) => rssi,
            Err(e) => {
                crate::dprintln!("✗ WiFi connect failed: {:?}", e);
                teardown(&mut controller).await;
                reset_tx_power(&mut tx_power, &mut success_streak);
                STATUS.lock().await.wifi_connected = false;
                Timer::after(RETRY_GAP).await;
                continue;
            }
        };
        crate::dprintln!("✓ Connected (RSSI: {})", connected_rssi);
        let connected_at = Instant::now();
        #[cfg(feature = "log-server")]
        {
            heap_steps.after_connect = heap();
        }

        // Disable modem power save for DHCP + the TLS handshake: MAX_MODEM's ~1s
        // sleep windows cause DNS UDP replies and TLS handshake packets to be
        // dropped (see git history). The radio is fully stopped after this
        // cycle's fetch anyway, so there's no "leave modem-sleep on between
        // fetches" phase to manage anymore — just make sure PS is off for now.
        let _ = controller.set_power_saving(PowerSaveMode::None);

        let dhcp_ok = with_timeout(DHCP_TIMEOUT, stack.wait_config_up())
            .await
            .is_ok();
        if !dhcp_ok {
            crate::dprintln!("✗ DHCP timed out");
            teardown(&mut controller).await;
            reset_tx_power(&mut tx_power, &mut success_streak);
            STATUS.lock().await.wifi_connected = false;
            Timer::after(RETRY_GAP).await;
            continue;
        }
        if let Some(config) = stack.config_v4() {
            crate::dprintln!("Network is up! IP: {}", config.address);
        }
        let dhcp_at = Instant::now();
        #[cfg(feature = "log-server")]
        {
            heap_steps.after_dhcp = heap();
        }

        let outcome = match with_timeout(
            FETCH_TIMEOUT,
            do_one_fetch(&tcp_client, &dns_socket, tls_read_buffer, tls_write_buffer),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                crate::dprintln!("✗ Fetch timed out after {}s", FETCH_TIMEOUT.as_secs());
                FetchOutcome::ConnectionLost
            }
        };

        #[cfg(feature = "log-server")]
        {
            let fetch_ok = matches!(outcome, FetchOutcome::Success);
            if fetch_ok {
                fetch_ok_total += 1;
            } else {
                fetch_fail_total += 1;
            }
            heap_steps.after_fetch = heap();
            let ms = |from: Instant, to: Instant| to.duration_since(from).as_millis() as u32;
            let report = super::telemetry::CycleReport {
                fetch_ok,
                fetch_ok_total,
                fetch_fail_total,
                connect_ms: ms(cycle_start, connected_at),
                dhcp_ms: ms(connected_at, dhcp_at),
                fetch_ms: ms(dhcp_at, Instant::now()),
                prev_radio_on_ms,
                heap: heap_steps,
            };
            super::telemetry::send_report(&tcp_client, &report).await;
        }

        // Always tear the radio all the way down before the idle gap — this is
        // the whole point of the redesign (see Power-Management.md).
        teardown(&mut controller).await;
        STATUS.lock().await.wifi_connected = false;
        #[cfg(feature = "log-server")]
        {
            prev_radio_on_ms = Instant::now().duration_since(cycle_start).as_millis() as u32;
            prev_heap_after_teardown = heap();
        }
        #[cfg(not(feature = "log-server"))]
        let _ = (cycle_start, connected_at, dhcp_at);

        let wait = match outcome {
            FetchOutcome::Success => {
                step_tx_power_on_success(&mut tx_power, &mut success_streak);
                crate::dprintln!(
                    "=== Fetch complete, sleeping {}s ===",
                    SUCCESS_GAP.as_secs()
                );
                SUCCESS_GAP
            }
            FetchOutcome::ConnectionLost => {
                reset_tx_power(&mut tx_power, &mut success_streak);
                crate::dprintln!("Reconnecting in {}s...", RETRY_GAP.as_secs());
                RETRY_GAP
            }
        };
        STATUS.lock().await.tx_power = tx_power;

        Timer::after(wait).await;
    }
}

/// Disconnects and stops the radio, tolerating "already stopped".
async fn teardown(controller: &mut WifiController<'static>) {
    if controller.is_started().unwrap_or(false) {
        let _ = controller.disconnect_async().await;
        let _ = controller.stop_async().await;
    }
    // Only let the CPU light-sleep once the radio is confirmed stopped; an
    // unreadable state counts as still running.
    RADIO_ACTIVE.store(controller.is_started().unwrap_or(true), Ordering::Release);
}

/// Lock TX power at max after any failure — same "this board needs the headroom
/// after trouble" behavior as the old design's `tx_locked` flag.
fn reset_tx_power(tx_power: &mut i8, success_streak: &mut u32) {
    *tx_power = wifi_connect::TX_POWER_MAX;
    *success_streak = 0;
}

/// After enough consecutive successful cycles, probe one step lower.
fn step_tx_power_on_success(tx_power: &mut i8, success_streak: &mut u32) {
    if *tx_power <= wifi_connect::TX_POWER_MIN {
        return;
    }
    *success_streak = success_streak.saturating_add(1);
    if *success_streak >= wifi_connect::SUCCESSES_BEFORE_STEP {
        *tx_power -= wifi_connect::TX_POWER_STEP;
        *success_streak = 0;
        crate::dprintln!(
            "TX power stepped down to {} ({:.1} dBm)",
            tx_power,
            *tx_power as f32 / 4.0
        );
    }
}

/// Opens a fresh TLS connection to `HOST_URL` and sends one GET, streaming +
/// parsing the response. There is no connection reuse across cycles anymore —
/// the radio is stopped between cycles, so nothing survives to reuse.
async fn do_one_fetch(
    tcp_client: &TcpClient<'static, 1, TCP_TX_SIZE, 32768>,
    dns_socket: &DnsSocket<'static>,
    tls_read_buffer: &mut [u8],
    tls_write_buffer: &mut [u8],
) -> FetchOutcome {
    let tls_config = TlsConfig::new(
        0, // seed for RNG
        tls_read_buffer,
        tls_write_buffer,
        TlsVerify::None, // Skip certificate verification for now
    );
    let mut client = HttpClient::new_with_tls(tcp_client, dns_socket, tls_config);

    crate::dprintln!("");
    crate::dprintln!("=== Connecting to {} (DNS+TCP+TLS) ===", HOST_URL);
    let mut resource = match with_timeout(Duration::from_secs(30), client.resource(HOST_URL)).await
    {
        Ok(Ok(r)) => {
            crate::dprintln!("✓ Handshake complete");
            r
        }
        Ok(Err(e)) => {
            crate::dprintln!("✗ Error connecting: {:?}", e);
            return FetchOutcome::ConnectionLost;
        }
        Err(_timeout) => {
            crate::dprintln!("✗ DNS+TCP+TLS handshake timed out after 30s");
            return FetchOutcome::ConnectionLost;
        }
    };
    crate::dprintln!("✓ Connected");

    crate::dprintln!("=== Fetching {} ===", PATH);
    fetch_once(&mut resource).await
}

/// Sends one GET over an already-connected `resource` and streams+parses the response.
/// Returns `ConnectionLost` on any transport-level error, signalling the caller to reconnect.
async fn fetch_once<C>(resource: &mut reqwless::client::HttpResource<'_, C>) -> FetchOutcome
where
    C: embedded_io_async::Read + embedded_io_async::Write,
{
    let mut headers_buf = [0u8; 1024];
    let mut station_data: [StationData; STATION_DATA_LEN] =
        [StationData::default(); STATION_DATA_LEN];

    let request = resource
        .get(PATH)
        .headers(&[("User-Agent", "ESP32-S3/1.0"), ("Accept", "*/*")]);

    let response = match request.send(&mut headers_buf).await {
        Ok(response) => response,
        Err(e) => {
            crate::dprintln!("✗ Error sending request: {:?}", e);
            return FetchOutcome::ConnectionLost;
        }
    };

    crate::dprintln!("✓ Status: {:?}", response.status);

    let mut body_reader = response.body().reader();
    let mut parser = StreamingStationParser::new(TARGET_STATIONS);

    let mut chunk_buf = [0u8; 1024]; // 1KB chunks
    let mut total_bytes = 0;
    let mut stations_found = 0;

    use embedded_io_async::Read as _;

    // Stream and parse chunks incrementally
    loop {
        match body_reader.read(&mut chunk_buf).await {
            Ok(0) => {
                crate::dprintln!("✓ Stream complete! Total: {} bytes", total_bytes);
                parser.finish();
                crate::dprintln!("✓ Found {} matching stations total", stations_found);
                STATION_DATA_SIGNAL.signal(station_data);
                STATUS.lock().await.last_fetch_at = Some(Instant::now());
                FETCH_SIGNAL.signal(Instant::now());
                return FetchOutcome::Success;
            }
            Ok(n) => {
                total_bytes += n;
                // Parse this chunk incrementally
                if let Ok(chunk_str) = core::str::from_utf8(&chunk_buf[..n]) {
                    let stations = parser.process_chunk(chunk_str);
                    stations_found += stations.len();

                    for station in stations.iter() {
                        if (station.station_idx as usize) < STATION_DATA_LEN {
                            station_data[station.station_idx as usize] = *station;
                        }
                    }
                } else {
                    crate::dprintln!("  Warning: Invalid UTF-8 in chunk");
                }
            }
            Err(e) => {
                crate::dprintln!("✗ Error reading: {:?}", e);
                return FetchOutcome::ConnectionLost;
            }
        }
    }
}
