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

use bay_wheels_controller::tasks::{blink, fetch, input_read, station_leds, wifi_connect};
use bay_wheels_controller::{network, provisioning, spi_devices, wifi, wifi_config};

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

/// Get WiFi credentials from NVS, returning None if none are stored.
fn get_wifi_credentials(flash: FlashStorage<'static>) -> Option<(String, String)> {
    match wifi_config::load_credentials(flash) {
        Ok(creds) => {
            rprintln!("Using WiFi credentials from NVS");
            Some((creds.ssid, creds.password))
        }
        Err(_) => None,
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

    let button_pressed = is_provisioning_button_pressed(&provision_button);
    drop(provision_button);

    // Enter provisioning if GPIO2 was held at boot.
    if button_pressed {
        rprintln!("Provisioning button pressed - entering WiFi setup mode");
        let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
        let flash = FlashStorage::new(peripherals.FLASH);
        provisioning::run_provisioning(usb_serial, flash);
        // Never returns
    }

    // Check credentials before initializing Embassy/WiFi so FLASH is still free.
    let creds = {
        let flash = FlashStorage::new(peripherals.FLASH);
        get_wifi_credentials(flash) // flash dropped after this block
    };

    let (ssid, password) = match creds {
        Some(c) => c,
        None => {
            // No credentials stored yet - enter provisioning automatically.
            rprintln!("No WiFi credentials found - entering setup mode automatically");
            let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
            // SAFETY: The FlashStorage created above has been dropped.
            // Flash operations use ROM functions and hold no exclusive hardware resources,
            // so stealing the peripheral for a fresh FlashStorage is safe here.
            let flash = FlashStorage::new(unsafe { esp_hal::peripherals::FLASH::steal() });
            provisioning::run_provisioning(usb_serial, flash);
            // Never returns
        }
    };

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
