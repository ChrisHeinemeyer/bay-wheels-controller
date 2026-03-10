use alloc::collections::btree_set::BTreeSet;
use alloc::string::{String, ToString};
use core::cell::RefCell;
use critical_section::Mutex;
use ieee80211::{match_frames, mgmt_frame::BeaconFrame};

static KNOWN_SSIDS: Mutex<RefCell<BTreeSet<String>>> = Mutex::new(RefCell::new(BTreeSet::new()));

pub fn setup_sniffer(mut sniffer: esp_radio::wifi::Sniffer) {
    sniffer.set_promiscuous_mode(true).unwrap();
    sniffer.set_receive_cb(|packet| {
        let _ = match_frames! {
            packet.data,
            beacon = BeaconFrame => {
                let Some(ssid) = beacon.ssid() else {
                    return;
                };
                if critical_section::with(|cs| {
                    KNOWN_SSIDS.borrow_ref_mut(cs).insert(ssid.to_string())
                }) {
                    crate::dprintln!("Found new AP with SSID: {}", ssid);
                }
            }
        };
    });
}
