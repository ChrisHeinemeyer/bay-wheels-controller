use super::station_parser::StreamingStationParser;
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Instant, Timer};
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::RequestBuilder,
};
use static_cell::StaticCell;

use esp_wifi_sys::include::{
    esp_wifi_set_ps, wifi_ps_type_t_WIFI_PS_MAX_MODEM, wifi_ps_type_t_WIFI_PS_NONE,
};

use crate::{
    stations::{STATION_DATA_LEN, TARGET_STATIONS},
    tasks::{
        signals::{FETCH_SIGNAL, STATION_DATA_SIGNAL, STATUS},
        station_parser::StationData,
    },
};

// Resource root: the TLS connection is established against this once and reused
// across repeated fetches (avoids paying for a fresh handshake every 60s).
const HOST_URL: &str = "https://gbfs.lyft.com";
const PATH: &str = "/gbfs/2.3/bay/en/station_status.json";

/// Outcome of one fetch attempt over an already-open connection.
enum FetchOutcome {
    /// Body streamed and parsed successfully.
    Success,
    /// The connection is now in an unknown state (write/read error) — reconnect from scratch.
    ConnectionLost,
}

#[embassy_executor::task]
pub async fn fetch_task(stack: &'static Stack<'static>) {
    // Wait for network to be ready
    crate::dprintln!("Fetch task: Waiting for network...");
    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        crate::dprintln!("Network is up! IP: {}", config.address);
    }

    // Create static buffers for TCP client state and TLS
    // TLS needs large buffers for handshake AND streaming - increase for large responses
    static TCP_CLIENT_STATE: StaticCell<TcpClientState<1, 32768, 32768>> = StaticCell::new();
    static TLS_RX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();
    static TLS_TX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();

    let tcp_state = TCP_CLIENT_STATE.init(TcpClientState::new());
    let tls_read_buffer = TLS_RX_BUFFER.init([0; 32768]);
    let tls_write_buffer = TLS_TX_BUFFER.init([0; 32768]);

    let tcp_client = TcpClient::new(*stack, tcp_state);
    let dns_socket = DnsSocket::new(*stack);

    loop {
        // Disable modem power save for the connect: MAX_MODEM's ~1s sleep windows cause
        // DNS UDP replies and TLS handshake packets to be dropped (see git history).
        unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) };

        let tls_config = TlsConfig::new(
            0, // seed for RNG
            tls_read_buffer,
            tls_write_buffer,
            TlsVerify::None, // Skip certificate verification for now
        );
        let mut client = HttpClient::new_with_tls(&tcp_client, &dns_socket, tls_config);

        crate::dprintln!("");
        crate::dprintln!("=== Connecting to {} ===", HOST_URL);
        let mut resource = match client.resource(HOST_URL).await {
            Ok(r) => r,
            Err(e) => {
                crate::dprintln!("✗ Error connecting: {:?}", e);
                unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_MAX_MODEM) };
                Timer::after(Duration::from_secs(60)).await;
                continue;
            }
        };
        crate::dprintln!("✓ Connected — reusing this connection until it drops");

        // Reuse this one connection for repeated fetches until something goes wrong (the
        // GBFS server is under no obligation to keep an idle connection open for 60s), at
        // which point we fall out to the outer loop and reconnect from scratch.
        loop {
            unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) };
            crate::dprintln!("=== Fetching {} ===", PATH);

            let outcome = fetch_once(&mut resource).await;

            unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_MAX_MODEM) };

            if let FetchOutcome::ConnectionLost = outcome {
                crate::dprintln!("Reconnecting in 60 seconds...");
                Timer::after(Duration::from_secs(60)).await;
                break;
            }

            crate::dprintln!("=== Request complete, waiting 60 seconds ===");
            crate::dprintln!("");
            Timer::after(Duration::from_secs(60)).await;
        }
    }
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
