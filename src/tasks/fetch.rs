use super::station_parser::StreamingStationParser;
use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Timer};
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::{Method, RequestBuilder},
};
use rtt_target::rprintln;
use static_cell::StaticCell;

// Start with a simple API that has a tiny response (~200 bytes)
// const URL: &str = "https://api.open-notify.org/astros.json";
const URL: &str = "https://gbfs.lyft.com/gbfs/2.3/bay/en/station_status.json";

// Station IDs we care about
static TARGET_STATIONS: &[&str] = &[
    "bfb90ed7-6039-4c61-9b13-fb60b1786dde",
    "f0083331-9bf8-407f-bba2-ab00c8968db9",
    // Add more station IDs here
];

#[embassy_executor::task]
pub async fn fetch_task(stack: &'static Stack<'static>) {
    // Wait for network to be ready
    rprintln!("Fetch task: Waiting for network...");
    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        rprintln!("Network is up! IP: {}", config.address);
    }

    // Create static buffers for TCP client state and TLS
    // TLS needs large buffers for handshake AND streaming - increase for large responses
    static TCP_CLIENT_STATE: StaticCell<TcpClientState<2, 32768, 32768>> = StaticCell::new();
    static TLS_RX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();
    static TLS_TX_BUFFER: StaticCell<[u8; 32768]> = StaticCell::new();

    let tcp_state = TCP_CLIENT_STATE.init(TcpClientState::new());
    let tls_read_buffer = TLS_RX_BUFFER.init([0; 32768]);
    let tls_write_buffer = TLS_TX_BUFFER.init([0; 32768]);

    // Create TCP client and DNS socket
    let tcp_client = TcpClient::new(*stack, tcp_state);
    let dns_socket = DnsSocket::new(*stack);

    loop {
        rprintln!("");
        rprintln!("=== Fetching from {} ===", URL);

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
                    request.headers(&[("User-Agent", "ESP32-C6/1.0"), ("Accept", "*/*")]);

                rprintln!("Sending HTTPS request...");

                // Small buffer just for HTTP headers/request metadata
                let mut headers_buf = [0u8; 1024];

                match request.send(&mut headers_buf).await {
                    Ok(response) => {
                        let status = response.status;
                        rprintln!("✓ Status: {:?}", status);

                        // Get a reader to stream the body
                        let mut body_reader = response.body().reader();

                        rprintln!("✓ Streaming and parsing response...");
                        rprintln!("  Looking for stations: {:?}", TARGET_STATIONS);

                        // Create streaming parser - processes chunks as they arrive
                        let mut parser = StreamingStationParser::new(TARGET_STATIONS);

                        let mut chunk_buf = [0u8; 4096]; // 4KB chunks
                        let mut total_bytes = 0;
                        let mut stations_found = 0;

                        use embedded_io_async::Read as _;

                        // Stream and parse chunks incrementally
                        loop {
                            match body_reader.read(&mut chunk_buf).await {
                                Ok(0) => {
                                    rprintln!("✓ Stream complete! Total: {} bytes", total_bytes);
                                    parser.finish();
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
                                            rprintln!(
                                                "  → {}: {} bikes, {} ebikes",
                                                station.station_id,
                                                station.num_bikes_available,
                                                station.num_ebikes_available
                                            );
                                        }
                                    } else {
                                        rprintln!("  Warning: Invalid UTF-8 in chunk");
                                    }
                                }
                                Err(e) => {
                                    rprintln!("✗ Error reading: {:?}", e);
                                    break;
                                }
                            }
                        }

                        rprintln!("✓ Found {} matching stations total", stations_found);
                    }
                    Err(e) => {
                        rprintln!("✗ Error sending request: {:?}", e);
                    }
                }
            }
            Err(e) => {
                rprintln!("✗ Error creating request: {:?}", e);
            }
        }

        rprintln!("=== Request complete, waiting 30 seconds ===");
        rprintln!("");
        Timer::after(Duration::from_secs(30)).await;
    }
}
