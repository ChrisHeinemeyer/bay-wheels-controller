use alloc::string::String;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{AuthMethod, PowerSaveMode, WifiController};
use esp_wifi_sys::include::esp_wifi_set_max_tx_power;

use crate::tasks::signals::STATUS;

/// Starting TX power (quarter-dBm). Used for initial connect and reconnects.
const TX_POWER_MAX: i8 = 84; // 21 dBm
/// Minimum TX power the adaptive logic will probe down to (user-tested safe floor).
const TX_POWER_MIN: i8 = 60; // 15.0 dBm
/// Step size for each downward probe (quarter-dBm = 0.5 dBm per step).
const TX_POWER_STEP: i8 = 2;
/// Number of 5-second maintenance ticks to stay stable before stepping down.
const STABLE_TICKS_PER_STEP: u32 = 36; // 36 × 5 s = 3 min per step

#[embassy_executor::task]
pub async fn wifi_connect_task(
    mut controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
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
            target_ap = Some(ap);
        }
    }

    let target_ap = target_ap.expect("Could not find target SSID in scan results");
    crate::dprintln!("");
    crate::dprintln!("Target AP details:");
    crate::dprintln!("  SSID: {}", target_ap.ssid);
    crate::dprintln!("  RSSI: {} dBm", target_ap.signal_strength);
    crate::dprintln!("  Channel: {:?}", target_ap.channel);
    crate::dprintln!("  Auth Method: {:?}", target_ap.auth_method);
    crate::dprintln!("");

    // Configure with scanned AP settings.
    // listen_interval=10: radio sleeps for 10 beacon intervals (~1 s) in MAX_MODEM power save.
    let client_config = esp_radio::wifi::ClientConfig::default()
        .with_ssid(String::from(ssid))
        .with_password(String::from(password))
        .with_auth_method(target_ap.auth_method.unwrap_or(AuthMethod::Wpa2Personal))
        .with_channel(target_ap.channel)
        .with_listen_interval(10);

    crate::dprintln!("Setting WiFi config for SSID: '{}'", ssid);
    crate::dprintln!("  Auth method: {:?}", target_ap.auth_method);
    crate::dprintln!("  Channel: {:?}", target_ap.channel);

    controller
        .set_config(&esp_radio::wifi::ModeConfig::Client(client_config))
        .unwrap();

    // Connect
    crate::dprintln!("Initiating connection...");
    match controller.connect() {
        Ok(_) => crate::dprintln!("Connect command sent successfully"),
        Err(e) => {
            crate::dprintln!("Connect command failed: {:?}", e);
            panic!("Failed to initiate connection");
        }
    }

    // Wait for connection with timeout. Some boards need disconnect+connect retries.
    let mut attempts = 0;
    let max_attempts = 5000; // 500 seconds total
    let retry_interval = 50; // Retry connect every 5 seconds
    let mut last_status = false;

    loop {
        let is_connected = controller.is_connected().unwrap_or(false);

        if is_connected != last_status {
            if is_connected {
                crate::dprintln!("✓ Status changed: CONNECTED");
            } else {
                crate::dprintln!("✗ Status changed: DISCONNECTED");
            }
            last_status = is_connected;
        }

        if is_connected {
            crate::dprintln!("✓ WiFi connected successfully!");
            {
                let mut guard = STATUS.lock().await;
                guard.wifi_connected = true;
                guard.rssi = target_ap.signal_strength as i8;
            }
            break;
        }

        attempts += 1;
        if attempts > max_attempts {
            crate::dprintln!(
                "✗ Failed to connect to WiFi after {} seconds",
                max_attempts / 10
            );
            crate::dprintln!("Possible reasons:");
            crate::dprintln!("  - Wrong password");
            crate::dprintln!("  - Weak signal (RSSI: {})", target_ap.signal_strength);
            crate::dprintln!("  - AP authentication issues");
            crate::dprintln!("  - Board-specific: try power cycle or re-flash");
            panic!("WiFi connection timeout");
        }

        // Retry connect periodically — some boards get stuck and need disconnect+connect
        if attempts > 0 && attempts % retry_interval == 0 {
            crate::dprintln!(
                "Retrying connect... ({}s elapsed) [RSSI: {}]",
                attempts / 10,
                target_ap.signal_strength
            );
            let _ = controller.disconnect();
            Timer::after(Duration::from_millis(500)).await;
            if controller.connect().is_ok() {
                crate::dprintln!("Connect command re-sent");
            }
        }

        Timer::after(Duration::from_millis(100)).await;
    }

    // Enable maximum modem power save — radio sleeps for listen_interval beacons (~1 s)
    // between AP check-ins. Safe because this device only makes outbound HTTPS fetches.
    if let Err(e) = controller.set_power_saving(PowerSaveMode::Maximum) {
        crate::dprintln!("Warning: failed to set modem power save: {:?}", e);
    } else {
        crate::dprintln!("Modem power save: Maximum (listen_interval=10, ~1 s sleep)");
    }

    // Adaptive TX power state.
    // Start at TX_POWER_MAX for safety, then probe downward in steps.
    // On disconnect while stepping, lock at TX_POWER_MAX for the rest of the session
    // (TX circuitry on some boards can't sustain lower power).
    let mut tx_power: i8 = TX_POWER_MAX;
    let mut tx_locked = false;
    let mut stable_ticks: u32 = 0;

    // Keep WiFi connection alive and manage adaptive TX power + live RSSI.
    loop {
        Timer::after(Duration::from_secs(5)).await;

        let connected = controller.is_connected().unwrap_or(false);

        if !connected {
            crate::dprintln!("⚠ WiFi disconnected! Attempting reconnect...");

            // If we were stepping down, lock TX at max — this board needs the headroom.
            if tx_power < TX_POWER_MAX {
                crate::dprintln!(
                    "  TX power was {} ({:.1} dBm) at disconnect — locking at max",
                    tx_power,
                    tx_power as f32 / 4.0
                );
                tx_locked = true;
            }
            tx_power = TX_POWER_MAX;
            stable_ticks = 0;
            apply_tx_power(tx_power);
            STATUS.lock().await.tx_power = tx_power;

            let _ = controller.connect();
            STATUS.lock().await.wifi_connected = false;
            continue;
        }

        // Connected — update live RSSI.
        let rssi = controller.rssi().unwrap_or(0) as i8;
        {
            let mut guard = STATUS.lock().await;
            guard.wifi_connected = true;
            guard.rssi = rssi;
        }

        // Step TX power down if not locked and not yet at floor.
        if !tx_locked && tx_power > TX_POWER_MIN {
            stable_ticks = stable_ticks.saturating_add(1);
            if stable_ticks >= STABLE_TICKS_PER_STEP {
                tx_power -= TX_POWER_STEP;
                stable_ticks = 0;
                apply_tx_power(tx_power);
                STATUS.lock().await.tx_power = tx_power;
                crate::dprintln!(
                    "TX power stepped down to {} ({:.1} dBm)",
                    tx_power,
                    tx_power as f32 / 4.0
                );
            }
        }
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
