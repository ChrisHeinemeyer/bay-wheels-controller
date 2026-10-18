use alloc::string::String;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{AuthMethod, WifiController, WifiError};
use esp_wifi_sys::include::esp_wifi_set_max_tx_power;

use crate::tasks::signals::STATUS;

/// Starting TX power (quarter-dBm). Used for initial connect and after any failure.
pub const TX_POWER_MAX: i8 = 60; // 15 dBm
/// Minimum TX power the adaptive logic will probe down to (user-tested safe floor).
pub const TX_POWER_MIN: i8 = 24; // 6.0 dBm
/// Step size for each downward probe (quarter-dBm = 0.5 dBm per step).
pub const TX_POWER_STEP: i8 = 2;
/// Consecutive successful connect+fetch cycles required before stepping TX power
/// down one notch. A single successful association is a weaker signal than the
/// old design's "stayed connected for 3 stable minutes", so require a streak
/// rather than stepping on the very first success.
pub const SUCCESSES_BEFORE_STEP: u32 = 3;

/// Access-point details discovered during the one-time boot scan.
pub struct ApInfo {
    pub auth_method: Option<AuthMethod>,
    pub channel: u8,
    pub signal_strength: i8,
}

/// Puts the controller into station mode, scans for `ssid`, and configures the
/// client with the scanned AP's channel/auth method. Call this once at boot —
/// every later reconnect (via [`connect`]) reuses this configuration and skips
/// re-scanning.
pub async fn setup(
    controller: &mut WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) -> ApInfo {
    controller.set_mode(esp_radio::wifi::WifiMode::Sta).unwrap();
    controller.start().unwrap();
    crate::dprintln!("WiFi controller started!");

    apply_tx_power(TX_POWER_MAX);

    // Scan for networks
    crate::dprintln!("Scanning for networks...");
    let scan_config = esp_radio::wifi::ScanConfig::default().with_scan_type(
        esp_radio::wifi::ScanTypeConfig::Active {
            min: core::time::Duration::from_millis(100),
            max: core::time::Duration::from_millis(100),
        },
    );

    let scan_result = controller
        .scan_with_config(scan_config)
        .expect("Failed to scan for networks");
    crate::dprintln!("Found {} networks", scan_result.len());

    // Find target AP and display details
    let mut target_ap = None;
    for ap in scan_result.iter() {
        crate::dprintln!(
            "  SSID: {}, RSSI: {}, Auth: {:?}, Channel: {:?}",
            ap.ssid,
            ap.signal_strength,
            ap.auth_method,
            ap.channel
        );
        if ap.ssid == ssid {
            target_ap = Some(ApInfo {
                auth_method: ap.auth_method,
                channel: ap.channel,
                signal_strength: ap.signal_strength,
            });
        }
    }

    let target_ap = target_ap.expect("Could not find target SSID in scan results");
    crate::dprintln!("");
    crate::dprintln!("Target AP details:");
    crate::dprintln!("  RSSI: {} dBm", target_ap.signal_strength);
    crate::dprintln!("  Channel: {:?}", target_ap.channel);
    crate::dprintln!("  Auth Method: {:?}", target_ap.auth_method);
    crate::dprintln!("");

    crate::dprintln!("Setting WiFi config for SSID: '{}'", ssid);
    crate::dprintln!("  Auth method: {:?}", target_ap.auth_method);
    crate::dprintln!("  Channel: {:?}", target_ap.channel);
    configure(controller, ssid, password, &target_ap).unwrap();

    target_ap
}

/// Applies the client configuration for the AP found by [`setup`].
fn configure(
    controller: &mut WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
    ap: &ApInfo,
) -> Result<(), WifiError> {
    // listen_interval=10: radio sleeps for 10 beacon intervals (~1 s) in MAX_MODEM power save.
    // This only matters while `fetch_task` briefly re-enables modem sleep between
    // connect and the first fetch on a cycle — the radio is fully stopped the rest
    // of the time, see Power-Management.md.
    let client_config = esp_radio::wifi::ClientConfig::default()
        .with_ssid(String::from(ssid))
        .with_password(String::from(password))
        .with_auth_method(ap.auth_method.unwrap_or(AuthMethod::Wpa2Personal))
        .with_channel(ap.channel)
        .with_listen_interval(10);
    controller.set_config(&esp_radio::wifi::ModeConfig::Client(client_config))
}

/// Creates a fresh controller after the previous one was dropped (which
/// de-initialises the WiFi driver and frees its timers), in station mode with
/// the AP found by [`setup`]. The embassy-net stack's `WifiDevice` keeps
/// working: it is only a handle onto esp-radio's global RX/TX queues.
pub fn recreate(
    radio_init: &'static esp_radio::Controller<'static>,
    ssid: &'static str,
    password: &'static str,
    ap: &ApInfo,
) -> Result<WifiController<'static>, WifiError> {
    // SAFETY: the WIFI peripheral's only other owner was the controller just
    // dropped; there is never more than one controller at a time.
    let wifi = unsafe { esp_hal::peripherals::WIFI::steal() };
    let (mut controller, _interfaces) =
        esp_radio::wifi::new(radio_init, wifi, Default::default())?;
    controller.set_mode(esp_radio::wifi::WifiMode::Sta)?;
    configure(&mut controller, ssid, password, ap)?;
    Ok(controller)
}

/// Connects (or reconnects) using the configuration already applied by [`setup`],
/// applying `tx_power` before associating. Returns the connected RSSI on success.
///
/// NOTE: the ~500s bound below was inherited from the original one-shot boot
/// connect; it's generous for a per-cycle reconnect (Stage 1 of the WiFi-teardown
/// redesign) and may be worth shortening once tuned on hardware.
pub async fn connect(
    controller: &mut WifiController<'static>,
    tx_power: i8,
) -> Result<i8, WifiError> {
    apply_tx_power(tx_power);

    crate::dprintln!("Initiating connection...");
    controller.connect()?;

    let mut attempts = 0;
    let max_attempts = 5000; // 500 seconds total
    let retry_interval = 50; // Retry connect every 5 seconds

    loop {
        let is_connected = controller.is_connected().unwrap_or(false);

        if is_connected {
            let rssi = controller.rssi().unwrap_or(0) as i8;
            crate::dprintln!("✓ WiFi connected successfully!");
            {
                let mut guard = STATUS.lock().await;
                guard.wifi_connected = true;
                guard.rssi = rssi;
                guard.tx_power = tx_power;
            }
            return Ok(rssi);
        }

        attempts += 1;
        if attempts > max_attempts {
            crate::dprintln!(
                "✗ Failed to connect to WiFi after {} seconds",
                max_attempts / 10
            );
            return Err(WifiError::Disconnected);
        }

        // Retry connect periodically — some boards get stuck and need disconnect+connect
        if attempts % retry_interval == 0 {
            crate::dprintln!("Retrying connect... ({}s elapsed)", attempts / 10);
            let _ = controller.disconnect();
            Timer::after(Duration::from_millis(500)).await;
            let _ = controller.connect();
        }

        Timer::after(Duration::from_millis(100)).await;
    }
}

fn apply_tx_power(power: i8) {
    unsafe {
        let ret = esp_wifi_set_max_tx_power(power);
        if ret != 0 {
            crate::dprintln!(
                "Warning: esp_wifi_set_max_tx_power({}) failed: {}",
                power,
                ret
            );
        }
    }
}
