//! Station data from the LAN GBFS mirror (`gbfs-mirror` feature).
//!
//! The Home Assistant add-on in `homeassistant/` keeps a copy of Lyft's
//! station_status feed and serves just the stations a board asks for. Over
//! plain HTTP to `GBFS_MIRROR` from `.env` that skips DNS, the TLS handshake
//! and ~270 KB of download, which is most of each cycle's radio-on time. The
//! board sends its own station IDs (from `BOARD_STATION_MAP` and
//! `TARGET_STATIONS`), so the add-on needs no board map and can't drift from
//! the firmware. Any failure falls back to the direct HTTPS fetch.

use core::fmt::Write as _;
use core::net::SocketAddr;

use embassy_net::tcp::client::TcpClient;
use embassy_time::{Duration, Instant};
use embedded_io_async::{Read as _, Write as _};
use embedded_nal_async::TcpConnect as _;

use super::fetch::{FetchOutcome, FetchTimings, StationSink};
use crate::stations::{BOARD_STATION_MAP, TARGET_STATIONS};
use crate::tasks::signals::BoardId;

const GBFS_MIRROR: &str = env!("GBFS_MIRROR");
/// Upper bound on connect + request + response. A healthy mirror answers in
/// well under a second; past this the cycle falls back to Lyft.
pub const MIRROR_TIMEOUT: Duration = Duration::from_secs(5);
/// The only keys `StreamingStationParser` reads.
const FIELDS: &str = "station_id,num_bikes_available,num_ebikes_available";

/// Request path for this board's stations. ~40 bytes per station ID.
pub type RequestPath = heapless::String<2560>;

/// Builds `/gbfs/station_status.json?fields=...&ids=a,b,...` for `board`.
/// Called once at boot.
pub fn request_path(board: BoardId) -> RequestPath {
    let mut path = RequestPath::new();
    let _ = write!(path, "/gbfs/station_status.json?fields={FIELDS}&ids=");
    let Some((_, cells)) = BOARD_STATION_MAP.iter().find(|(b, _)| *b == board) else {
        return path;
    };
    let mut first = true;
    for (i, (_, idx)) in cells.iter().enumerate() {
        // A station can sit at more than one grid cell; ask for it once.
        if cells[..i].iter().any(|(_, earlier)| earlier == idx) {
            continue;
        }
        for (id, _) in TARGET_STATIONS.iter().filter(|(_, t)| t == idx) {
            let sep = if first { "" } else { "," };
            if write!(path, "{sep}{id}").is_err() {
                crate::dprintln!("✗ GBFS mirror: station list too long, truncated");
                return path;
            }
            first = false;
        }
    }
    path
}

/// One plain-HTTP GET of `path` from the mirror, streamed through the parser.
/// `Ok` only once the stations are published; the caller falls back otherwise.
pub async fn fetch<const N: usize, const TX: usize, const RX: usize>(
    tcp: &TcpClient<'static, N, TX, RX>,
    path: &str,
    timings: &mut FetchTimings,
) -> Result<(), &'static str> {
    let addr: SocketAddr = GBFS_MIRROR.parse().map_err(|_| "bad GBFS_MIRROR")?;
    let connect_start = Instant::now();
    let mut conn = tcp.connect(addr).await.map_err(|_| "connect")?;
    timings.handshake_ms = ms_since(connect_start);

    let request_start = Instant::now();
    let mut head = heapless::String::<96>::new();
    write!(
        head,
        " HTTP/1.1\r\nHost: {GBFS_MIRROR}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|_| "header")?;
    conn.write_all(b"GET ").await.map_err(|_| "write")?;
    conn.write_all(path.as_bytes()).await.map_err(|_| "write")?;
    conn.write_all(head.as_bytes()).await.map_err(|_| "write")?;
    conn.flush().await.map_err(|_| "flush")?;

    // Read up to the end of the headers; whatever follows is body.
    let mut buf = [0u8; 1024];
    let mut got = 0;
    let body_start = loop {
        let n = conn.read(&mut buf[got..]).await.map_err(|_| "read")?;
        if n == 0 {
            return Err("closed in headers");
        }
        got += n;
        if let Some(p) = buf[..got].windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if got == buf.len() {
            return Err("headers too long");
        }
    };
    // "HTTP/1.x 2xx"
    if buf.get(9) != Some(&b'2') {
        return Err("non-2xx (mirror stale?)");
    }
    timings.ttfb_ms = ms_since(request_start);

    // The server closes the connection after the body (HTTP/1.0 response).
    let mut sink = StationSink::new();
    sink.feed(&buf[body_start..got]);
    loop {
        match conn.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => sink.feed(&buf[..n]),
            Err(_) => return Err("read body"),
        }
    }
    match sink.finish(timings).await {
        FetchOutcome::Success => Ok(()),
        FetchOutcome::ConnectionLost => Err("parse"),
    }
}

fn ms_since(from: Instant) -> u32 {
    Instant::now().duration_since(from).as_millis() as u32
}
