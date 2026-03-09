#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{clock::CpuClock, gpio, timer::timg::TimerGroup, usb_serial_jtag::UsbSerialJtag};
use esp_storage::FlashStorage;
use rtt_target::rprintln;

use esp32_embasssy_wifi_test::tasks::{blink, fetch, input_read, station_leds, wifi_connect};
use esp32_embasssy_wifi_test::{network, provisioning, spi_devices, wifi, wifi_config};

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Check if the provisioning button (GPIO2) is pressed
fn is_provisioning_button_pressed(button: &gpio::Input<'_>) -> bool {
    // Button is active low (pressed = low)
    button.is_low()
}

/// Get WiFi credentials from NVS
/// Returns None if no credentials are stored - user must run provisioning first
fn get_wifi_credentials(flash: FlashStorage<'static>) -> Option<(String, String)> {
    match wifi_config::load_credentials(flash) {
        Ok(creds) => {
            rprintln!("Using WiFi credentials from NVS");
            Some((creds.ssid, creds.password))
        }
        Err(_) => {
            rprintln!("No WiFi credentials found in NVS");
            rprintln!("Hold GPIO2 button during boot to enter provisioning mode");
            None
        }
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Initialize RTT for logging
    rtt_target::rtt_init_print!();
    rprintln!("Starting ESP32-S3...");

    // Initialize ESP-HAL with max CPU clock
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Setup heap allocator
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    // Check provisioning button (GPIO2) before initializing Embassy
    let provision_button = gpio::Input::new(
        peripherals.GPIO2,
        gpio::InputConfig::default().with_pull(gpio::Pull::Up),
    );

    if is_provisioning_button_pressed(&provision_button) {
        rprintln!("Provisioning button pressed - entering WiFi setup mode");

        // Initialize USB-Serial-JTAG for provisioning
        let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
        let flash = FlashStorage::new(peripherals.FLASH);

        // Run provisioning (this will reboot when done)
        provisioning::run_provisioning(usb_serial, flash);
        // Never returns - reboots after provisioning
    }

    // Drop the button input so GPIO2 can be used elsewhere if needed
    drop(provision_button);

    // Initialize Embassy executor
    let timg0 = TimerGroup::new(peripherals.TIMG1);
    esp_rtos::start(timg0.timer0);
    rprintln!("Embassy initialized!");

    // Initialize WiFi radio
    let radio_init_value = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    let radio_init: &'static _ = Box::leak(Box::new(radio_init_value));
    let (controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    // Get WiFi credentials from NVS
    let flash = FlashStorage::new(peripherals.FLASH);
    let creds = get_wifi_credentials(flash);

    // If no credentials, panic with instructions (can't re-enter provisioning here
    // since we've already consumed FLASH - user must reboot with button held)
    let (ssid, password) = creds.unwrap_or_else(|| {
        rprintln!("===========================================");
        rprintln!("No WiFi credentials configured!");
        rprintln!("Reboot while holding GPIO2 button to provision.");
        rprintln!("===========================================");
        panic!("No WiFi credentials");
    });
    let ssid: &'static str = Box::leak(ssid.into_boxed_str());
    let password: &'static str = Box::leak(password.into_boxed_str());

    // Spawn WiFi connection task
    spawner
        .spawn(wifi_connect::wifi_connect_task(controller, ssid, password))
        .expect("Failed to spawn wifi_connect_task");

    // Setup embassy-net stack for networking
    let (stack, runner) = network::create_network_stack(interfaces.sta, network::dhcp_config());

    // Spawn network runner task to handle the stack
    spawner
        .spawn(net_task(runner))
        .expect("Failed to spawn net_task");

    // Setup WiFi sniffer
    wifi::setup_sniffer(interfaces.sniffer);

    // Setup LED on GPIO15
    let led_pin = gpio::Output::new(
        peripherals.GPIO4,
        gpio::Level::Low,
        gpio::OutputConfig::default(),
    );
    let clock_cfg =
        esp_hal::mcpwm::PeripheralClockConfig::with_frequency(esp_hal::time::Rate::from_mhz(40))
            .unwrap();
    let mut mcpwm = esp_hal::mcpwm::McPwm::new(peripherals.MCPWM0, clock_cfg);
    mcpwm.operator0.set_timer(&mcpwm.timer0);
    let pwm_pin = mcpwm.operator0.with_pin_a(
        led_pin,
        esp_hal::mcpwm::operator::PwmPinConfig::UP_ACTIVE_HIGH,
    );

    let timer_clock_cfg = clock_cfg
        .timer_clock_with_frequency(
            99,
            esp_hal::mcpwm::timer::PwmWorkingMode::Increase,
            esp_hal::time::Rate::from_khz(20),
        )
        .unwrap();
    mcpwm.timer0.start(timer_clock_cfg);

    let shift_register = spi_devices::shift_register::ShiftRegister::new(
        peripherals.SPI2,
        gpio::Output::new(
            peripherals.GPIO33,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Output::new(
            peripherals.GPIO35,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Input::new(peripherals.GPIO36, gpio::InputConfig::default()),
    );

    let al5887 = spi_devices::al5887::al5887::Al5887::new(
        peripherals.SPI3,
        gpio::Output::new(
            peripherals.GPIO10,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Output::new(
            peripherals.GPIO12,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Input::new(peripherals.GPIO13, gpio::InputConfig::default()),
        gpio::Output::new(
            peripherals.GPIO11,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Output::new(
            peripherals.GPIO8,
            gpio::Level::Low,
            gpio::OutputConfig::default(),
        ),
        gpio::Output::new(
            peripherals.GPIO9,
            gpio::Level::High,
            gpio::OutputConfig::default(),
        ),
    );

    // Spawn tasks
    rprintln!("Spawning tasks...");
    spawner
        .spawn(blink::blink_task(pwm_pin))
        .expect("Failed to spawn blink_task");
    spawner
        .spawn(station_leds::station_leds_task(al5887))
        .expect("Failed to spawn station_leds_task");
    spawner
        .spawn(fetch::fetch_task(stack))
        .expect("Failed to spawn fetch_task");
    spawner
        .spawn(input_read::input_read_task(shift_register))
        .expect("Failed to spawn input_read_task");
    rprintln!("All tasks spawned!");

    // Main loop - keep alive
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>) {
    runner.run().await
}
