//! WiFi configuration storage using NVS (Non-Volatile Storage)
//!
//! Stores and retrieves WiFi credentials from flash memory.

use alloc::string::String;
use esp_nvs::platform::EspFlash;
use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;
use rtt_target::rprintln;

/// NVS partition offset - adjust based on your partition table
/// Default ESP-IDF NVS partition starts at 0x9000
const NVS_PARTITION_OFFSET: usize = 0x9000;
/// NVS partition size (24KB default)
const NVS_PARTITION_SIZE: usize = 0x6000;

/// WiFi credentials
#[derive(Debug, Clone)]
pub struct WifiCredentials {
    pub ssid: String,
    pub password: String,
}

/// Error type for WiFi config operations
#[derive(Debug)]
pub enum WifiConfigError {
    NvsError,
    NotFound,
}

/// Load WiFi credentials from NVS
pub fn load_credentials(flash: FlashStorage<'static>) -> Result<WifiCredentials, WifiConfigError> {
    let mut platform = EspFlash::new(flash);
    let mut nvs =
        Nvs::new(NVS_PARTITION_OFFSET, NVS_PARTITION_SIZE, &mut platform).map_err(|e| {
            rprintln!("NVS init error: {:?}", e);
            WifiConfigError::NvsError
        })?;

    let namespace = Key::from_str("wifi");
    let ssid_key = Key::from_str("ssid");
    let password_key = Key::from_str("password");

    let ssid: String = nvs.get(&namespace, &ssid_key).map_err(|e| {
        rprintln!("Failed to read SSID: {:?}", e);
        WifiConfigError::NotFound
    })?;

    let password: String = nvs.get(&namespace, &password_key).map_err(|e| {
        rprintln!("Failed to read password: {:?}", e);
        WifiConfigError::NotFound
    })?;

    Ok(WifiCredentials { ssid, password })
}

/// Save WiFi credentials to NVS
pub fn save_credentials(
    flash: FlashStorage<'static>,
    ssid: &str,
    password: &str,
) -> Result<(), WifiConfigError> {
    let mut platform = EspFlash::new(flash);
    let mut nvs =
        Nvs::new(NVS_PARTITION_OFFSET, NVS_PARTITION_SIZE, &mut platform).map_err(|e| {
            rprintln!("NVS init error: {:?}", e);
            WifiConfigError::NvsError
        })?;

    let namespace = Key::from_str("wifi");
    let ssid_key = Key::from_str("ssid");
    let password_key = Key::from_str("password");

    nvs.set(&namespace, &ssid_key, ssid).map_err(|e| {
        rprintln!("Failed to write SSID: {:?}", e);
        WifiConfigError::NvsError
    })?;

    nvs.set(&namespace, &password_key, password).map_err(|e| {
        rprintln!("Failed to write password: {:?}", e);
        WifiConfigError::NvsError
    })?;

    rprintln!("WiFi credentials saved to NVS");
    Ok(())
}
