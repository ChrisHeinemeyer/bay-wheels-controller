use super::station_parser::StreamingStationParser;
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Instant, Timer};
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::{Method, RequestBuilder},
};
use static_cell::StaticCell;

use crate::{
    stations::{STATION_DATA_LEN, TARGET_STATIONS},
    tasks::{
        signals::{STATION_DATA_SIGNAL, STATUS},
        station_parser::StationData,
    },
};

// Start with a simple API that has a tiny response (~200 bytes)
// const URL: &str = "https://api.open-notify.org/astros.json";
const URL: &str = "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json";
#[embassy_executor::task]
pub async fn fetch_task(stack: &'static Stack<'static>) {
    // Wait for network to be ready
    crate::dprintln!("Fetch task: Waiting for network...");
    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        crate::dprintln!("Network is up! IP: {}", config.address);
    }

    crate::dprintln!("DEBUG: Starting to initialize TCP client state...");

    // Create static buffers for TCP client state and TLS
    // TLS needs large buffers for handshake AND streaming - increase for large responses
    static TCP_CLIENT_STATE: StaticCell<TcpClientState<1, 32768, 32768>> = StaticCell::new();
    static TLS_RX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();
    static TLS_TX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();

    crate::dprintln!("DEBUG: Initializing TCP state...");
    let tcp_state = TCP_CLIENT_STATE.init(TcpClientState::new());
    let tls_read_buffer = TLS_RX_BUFFER.init([0; 32768]);
    let tls_write_buffer = TLS_TX_BUFFER.init([0; 32768]);

    // Create TCP client and DNS socket
    crate::dprintln!("DEBUG: Creating TCP client...");
    let tcp_client = TcpClient::new(*stack, tcp_state);
    crate::dprintln!("DEBUG: TCP client created");

    crate::dprintln!("DEBUG: Creating DNS socket...");
    let dns_socket = DnsSocket::new(*stack);
    crate::dprintln!("DEBUG: DNS socket created");

    crate::dprintln!("DEBUG: Entering main loop...");

    loop {
        crate::dprintln!("");
        crate::dprintln!("=== Fetching from {} ===", URL);

        // Create TLS config (using a simple seed based on system time would be better, but using 0 for now)
        let tls_config = TlsConfig::new(
            0, // seed for RNG
            tls_read_buffer,
            tls_write_buffer,
            TlsVerify::None, // Skip certificate verification for now
        );

        // Create HTTP client with TLS
        let mut client = HttpClient::new_with_tls(&tcp_client, &dns_socket, tls_config);

        // Create request
        match client.request(Method::GET, URL).await {
            Ok(request) => {
                // Add headers
                let mut request =
                    request.headers(&[("User-Agent", "ESP32-S3/1.0"), ("Accept", "*/*")]);

                crate::dprintln!("Sending HTTPS request...");

                // Small buffer just for HTTP headers/request metadata
                let mut headers_buf = [0u8; 1024];
                let mut station_data: [StationData; STATION_DATA_LEN] =
                    [StationData::default(); STATION_DATA_LEN];
                match request.send(&mut headers_buf).await {
                    Ok(response) => {
                        let status = response.status;
                        crate::dprintln!("✓ Status: {:?}", status);

                        // Get a reader to stream the body
                        let mut body_reader = response.body().reader();

                        // Create streaming parser - processes chunks as they arrive
                        let mut parser = StreamingStationParser::new(TARGET_STATIONS);

                        let mut chunk_buf = [0u8; 1024]; // 1KB chunks
                        let mut total_bytes = 0;
                        let mut stations_found = 0;

                        use embedded_io_async::Read as _;

                        // Stream and parse chunks incrementally
                        loop {
                            match body_reader.read(&mut chunk_buf).await {
                                Ok(0) => {
                                    crate::dprintln!(
                                        "✓ Stream complete! Total: {} bytes",
                                        total_bytes
                                    );
                                    parser.finish();
                                    STATION_DATA_SIGNAL.signal(station_data);
                                    STATUS.lock().await.last_fetch_at = Some(Instant::now());
                                    break;
                                }
                                Ok(n) => {
                                    total_bytes += n;
                                    // Parse this chunk incrementally
                                    if let Ok(chunk_str) = core::str::from_utf8(&chunk_buf[..n]) {
                                        let stations = parser.process_chunk(chunk_str);
                                        stations_found += stations.len();

                                        // Print any stations found in this chunk
                                        for station in stations.iter() {
                                            if (station.station_idx as usize) < STATION_DATA_LEN {
                                                station_data[station.station_idx as usize] =
                                                    *station;
                                            }
                                        }
                                    } else {
                                        crate::dprintln!("  Warning: Invalid UTF-8 in chunk");
                                    }
                                }
                                Err(e) => {
                                    crate::dprintln!("✗ Error reading: {:?}", e);
                                    break;
                                }
                            }
                        }

                        crate::dprintln!("✓ Found {} matching stations total", stations_found);
                    }
                    Err(e) => {
                        crate::dprintln!("✗ Error sending request: {:?}", e);
                    }
                }
            }
            Err(e) => {
                crate::dprintln!("✗ Error creating request: {:?}", e);
            }
        }

        crate::dprintln!("=== Request complete, waiting 30 seconds ===");
        crate::dprintln!("");
        Timer::after(Duration::from_secs(30)).await;
    }
}
