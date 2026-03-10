use alloc::string::String;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{AuthMethod, WifiController};

use crate::tasks::signals::STATUS;

#[embassy_executor::task]
pub async fn wifi_connect_task(
    mut controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
    controller.set_mode(esp_radio::wifi::WifiMode::Sta).unwrap();
    controller.start().unwrap();
    crate::dprintln!("WiFi controller started!");

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

    // Configure with scanned AP settings
    let client_config = esp_radio::wifi::ClientConfig::default()
        .with_ssid(String::from(ssid))
        .with_password(String::from(password))
        .with_auth_method(target_ap.auth_method.unwrap_or(AuthMethod::Wpa2Personal))
        .with_channel(target_ap.channel);

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

    // Wait for connection with timeout
    let mut attempts = 0;
    let max_attempts = 150; // 15 seconds
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
            panic!("WiFi connection timeout");
        }

        if attempts % 10 == 0 {
            crate::dprintln!(
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
            crate::dprintln!("⚠ WiFi disconnected! Attempting reconnect...");
            let _ = controller.connect();
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}
