use alloc::string::String;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{AuthMethod, WifiController};
use rtt_target::rprintln;

use crate::tasks::signals::STATUS;

#[embassy_executor::task]
pub async fn wifi_connect_task(
    mut controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
    controller.set_mode(esp_radio::wifi::WifiMode::Sta).unwrap();
    controller.start().unwrap();
    rprintln!("WiFi controller started!");

    // Scan for networks
    rprintln!("Scanning for networks...");
    let scan_config = esp_radio::wifi::ScanConfig::default().with_scan_type(
        esp_radio::wifi::ScanTypeConfig::Active {
            min: core::time::Duration::from_millis(100),
            max: core::time::Duration::from_millis(100),
        },
    );

    let scan_result = controller
        .scan_with_config(scan_config)
        .expect("Failed to scan for networks");
    rprintln!("Found {} networks", scan_result.len());

    // Find target AP and display details
    let mut target_ap = None;
    for ap in scan_result.iter() {
        rprintln!(
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
    rprintln!("");
    rprintln!("Target AP details:");
    rprintln!("  SSID: {}", target_ap.ssid);
    rprintln!("  RSSI: {} dBm", target_ap.signal_strength);
    rprintln!("  Channel: {:?}", target_ap.channel);
    rprintln!("  Auth Method: {:?}", target_ap.auth_method);
    rprintln!("");

    // Configure with scanned AP settings
    let client_config = esp_radio::wifi::ClientConfig::default()
        .with_ssid(String::from(ssid))
        .with_password(String::from(password))
        .with_auth_method(target_ap.auth_method.unwrap_or(AuthMethod::Wpa2Personal))
        .with_channel(target_ap.channel);

    rprintln!("Setting WiFi config for SSID: '{}'", ssid);
    rprintln!("  Auth method: {:?}", target_ap.auth_method);
    rprintln!("  Channel: {:?}", target_ap.channel);

    controller
        .set_config(&esp_radio::wifi::ModeConfig::Client(client_config))
        .unwrap();

    // Connect
    rprintln!("Initiating connection...");
    match controller.connect() {
        Ok(_) => rprintln!("Connect command sent successfully"),
        Err(e) => {
            rprintln!("Connect command failed: {:?}", e);
            panic!("Failed to initiate connection");
        }
    }

    // Wait for connection with timeout
    let mut attempts = 0;
    let max_attempts = 150; // 15 seconds
    let mut last_status = false;

    loop {
        let is_connected = controller.is_connected().unwrap_or(false);

        if is_connected != last_status {
            if is_connected {
                rprintln!("✓ Status changed: CONNECTED");
            } else {
                rprintln!("✗ Status changed: DISCONNECTED");
            }
            last_status = is_connected;
        }

        if is_connected {
            rprintln!("✓ WiFi connected successfully!");
            {
                let mut guard = STATUS.lock().await;
                guard.wifi_connected = true;
                guard.rssi = target_ap.signal_strength as i8;
            }
            break;
        }

        attempts += 1;
        if attempts > max_attempts {
            rprintln!(
                "✗ Failed to connect to WiFi after {} seconds",
                max_attempts / 10
            );
            rprintln!("Possible reasons:");
            rprintln!("  - Wrong password");
            rprintln!("  - Weak signal (RSSI: {})", target_ap.signal_strength);
            rprintln!("  - AP authentication issues");
            panic!("WiFi connection timeout");
        }

        if attempts % 10 == 0 {
            rprintln!(
                "Waiting for connection... ({}s) [RSSI: {}]",
                attempts / 10,
                target_ap.signal_strength
            );
        }

        Timer::after(Duration::from_millis(100)).await;
    }

    // Keep WiFi connection alive
    loop {
        let connected = controller.is_connected().unwrap_or(false);
        STATUS.lock().await.wifi_connected = connected;
        if !connected {
            rprintln!("⚠ WiFi disconnected! Attempting reconnect...");
            let _ = controller.connect();
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}
