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

use embassy_futures::select::{Either, select};
use embassy_net::tcp::client::TcpClient;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpAddress, IpEndpoint, Stack};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::{Read as _, Write as _};
use embedded_nal_async::TcpConnect as _;

use super::fetch::{FetchOutcome, FetchTimings, StationSink};
use crate::stations::{BOARD_STATION_MAP, TARGET_STATIONS};
use crate::tasks::signals::BoardId;

const GBFS_MIRROR: &str = env!("GBFS_MIRROR");
/// Upper bound on connect + request + response. A healthy mirror answers in
/// well under a second; past this the cycle falls back to Lyft.
pub const MIRROR_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one TCP connect attempt to the mirror may take before a new one.
/// The mirror is on the LAN, so a healthy connect takes ~20 ms.
const CONNECT_RETRY: Duration = Duration::from_millis(250);
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
    stack: Stack<'static>,
    tcp: &TcpClient<'static, N, TX, RX>,
    path: &str,
    timings: &mut FetchTimings,
) -> Result<(), &'static str> {
    let addr: SocketAddr = GBFS_MIRROR.parse().map_err(|_| "bad GBFS_MIRROR")?;
    let connect_start = Instant::now();
    // Right after association nothing gets through for a second or two, and
    // TCP's own SYN retransmit waits 1 s, then 2 s. A fresh connect attempt
    // every CONNECT_RETRY gets in as soon as the link passes frames, and
    // `handshake_ms` then measures that dead time.
    let mut rx = RxWatch::new(connect_start);
    let mut probe = Probe::new(stack, addr, connect_start);
    let mut conn = loop {
        let watch = select(rx.run(timings), probe.run());
        match with_timeout(CONNECT_RETRY, select(tcp.connect(addr), watch)).await {
            Ok(Either::First(Ok(conn))) => break conn,
            Ok(Either::First(Err(_))) => return Err("connect"),
            Ok(Either::Second(Either::First(never) | Either::Second(never))) => match never {},
            Err(_) => continue, // the outer MIRROR_TIMEOUT bounds this
        }
    };
    rx.check(timings);
    timings.first_echo = probe.first_echo;
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

/// Notes when the first frames arrive after the connect starts, to tell a
/// link that receives nothing from one where only our own traffic is lost.
struct RxWatch {
    start: Instant,
    group0: u32,
    unicast0: u32,
}

impl RxWatch {
    fn new(start: Instant) -> Self {
        let (group0, unicast0) = rx_frames();
        Self {
            start,
            group0,
            unicast0,
        }
    }

    fn check(&self, timings: &mut FetchTimings) {
        let (group, unicast) = rx_frames();
        if group != self.group0 && timings.first_group_rx_ms.is_none() {
            timings.first_group_rx_ms = Some(ms_since(self.start));
        }
        if unicast != self.unicast0 && timings.first_unicast_rx_ms.is_none() {
            timings.first_unicast_rx_ms = Some(ms_since(self.start));
        }
    }

    async fn run(&mut self, timings: &mut FetchTimings) -> core::convert::Infallible {
        loop {
            self.check(timings);
            Timer::after_millis(5).await;
        }
    }
}

/// How often a UDP probe goes to the mirror host during the connect.
const PROBE_INTERVAL: Duration = Duration::from_millis(20);

/// Numbered UDP probes to the mirror host's port, which logs their arrival
/// and echoes them back. The host's log shows when our uplink starts passing
/// frames, the first echo when the round trip does.
struct Probe {
    socket: UdpSocket<'static>,
    dest: IpEndpoint,
    start: Instant,
    seq: u16,
    cycle: u16,
    first_echo: Option<(u32, u16)>,
}

impl Probe {
    fn new(stack: Stack<'static>, dest: SocketAddr, start: Instant) -> Self {
        // Only one fetch cycle runs at a time, so these buffers are never
        // shared; the socket is dropped (and the slot freed) with the Probe.
        static mut RX_META: [PacketMetadata; 8] = [PacketMetadata::EMPTY; 8];
        static mut TX_META: [PacketMetadata; 8] = [PacketMetadata::EMPTY; 8];
        static mut RX_BUF: [u8; 256] = [0; 256];
        static mut TX_BUF: [u8; 256] = [0; 256];
        static CYCLE: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);
        #[allow(static_mut_refs)]
        let mut socket =
            unsafe { UdpSocket::new(stack, &mut RX_META, &mut RX_BUF, &mut TX_META, &mut TX_BUF) };
        let _ = socket.bind(40_000);
        let dest = match dest {
            SocketAddr::V4(v4) => IpEndpoint::new(IpAddress::Ipv4(*v4.ip()), v4.port()),
            SocketAddr::V6(_) => unreachable!("GBFS_MIRROR is an IPv4 address"),
        };
        Self {
            socket,
            dest,
            start,
            seq: 0,
            cycle: CYCLE.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            first_echo: None,
        }
    }

    async fn run(&mut self) -> core::convert::Infallible {
        loop {
            self.step(PROBE_INTERVAL).await;
        }
    }

    /// Sends one probe, then collects echoes for `interval`.
    async fn step(&mut self, interval: Duration) {
        let mut buf = [0u8; 16];
        // "BWP" cycle seq
        let mut msg = [b'B', b'W', b'P', 0, 0, 0, 0];
        msg[3..5].copy_from_slice(&self.cycle.to_be_bytes());
        msg[5..7].copy_from_slice(&self.seq.to_be_bytes());
        let _ = self.socket.send_to(&msg, self.dest).await;
        self.seq = self.seq.wrapping_add(1);
        let next = Instant::now() + interval;
        while let Ok(Ok((n, _))) =
            embassy_time::with_deadline(next, self.socket.recv_from(&mut buf)).await
        {
            if self.first_echo.is_none() && n >= 7 && buf[..3] == *b"BWP" {
                let seq = u16::from_be_bytes([buf[5], buf[6]]);
                self.first_echo = Some((ms_since(self.start), seq));
            }
        }
    }
}

/// How often `wait_for_downlink` probes while the radio dozes. Each send or
/// receive keeps the radio awake for a while, so probing too often leaves no
/// time to doze (at 100 ms the radio barely slept).
const DOZE_PROBE_INTERVAL: Duration =
    Duration::from_millis(crate::power::parse_u32(option_env!("DOZE_PROBE_MS"), 500) as u64);

/// Probes the mirror host until an echo comes back, i.e. until unicast toward
/// us gets through, or `limit` passes. Meant to run with WiFi power save on,
/// so the 1-3 s after a re-association in which the AP's unicast to us is
/// lost costs modem-sleep current instead of full receive current. Returns
/// the ms until the first echo.
pub async fn wait_for_downlink(stack: Stack<'static>, limit: Duration) -> Option<u32> {
    let addr: SocketAddr = GBFS_MIRROR.parse().ok()?;
    let mut probe = Probe::new(stack, addr, Instant::now());
    let _ = with_timeout(limit, async {
        while probe.first_echo.is_none() {
            probe.step(DOZE_PROBE_INTERVAL).await;
        }
    })
    .await;
    probe.first_echo.map(|(ms, _)| ms)
}

fn rx_frames() -> (u32, u32) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        smoltcp::iface::RX_GROUP_FRAMES.load(Relaxed),
        smoltcp::iface::RX_UNICAST_FRAMES.load(Relaxed),
    )
}

fn ms_since(from: Instant) -> u32 {
    Instant::now().duration_since(from).as_millis() as u32
}
