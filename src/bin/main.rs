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
#[cfg(not(feature = "debug-serial"))]
use bay_wheels_controller::tasks::serial_status;
use bay_wheels_controller::tasks::signals::{BoardId, STATUS};
use bay_wheels_controller::tasks::{blink, fetch, input_read, station_leds, wifi_connect};
use bay_wheels_controller::{GIT_VERSION, dprintln};
use bay_wheels_controller::{network, provisioning, spi_devices, wifi, wifi_config};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{clock::CpuClock, gpio, timer::timg::TimerGroup, usb_serial_jtag::UsbSerialJtag};
use esp_storage::FlashStorage;

// App descriptor with git version for binary inspection (Flash tab, esptool image-info).
esp_bootloader_esp_idf::esp_app_desc!(
    env!("GIT_VERSION"),
    env!("CARGO_PKG_NAME"),
    esp_bootloader_esp_idf::BUILD_TIME,
    esp_bootloader_esp_idf::BUILD_DATE,
    esp_bootloader_esp_idf::ESP_IDF_COMPATIBLE_VERSION,
    esp_bootloader_esp_idf::MMU_PAGE_SIZE,
    0,
    u16::MAX
);

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
#[cfg(not(feature = "use-env"))]
fn get_wifi_credentials(flash: FlashStorage<'static>) -> Option<(String, String)> {
    match wifi_config::load_credentials(flash) {
        Ok(creds) => {
            dprintln!("Using WiFi credentials from NVS");
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
    // RTT is only needed when debug-serial is off.
    #[cfg(not(feature = "debug-serial"))]
    rtt_target::rtt_init_print!();
    dprintln!("Starting ESP32-S3... FW {}", GIT_VERSION);

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
        dprintln!("Provisioning button pressed - entering WiFi setup mode");
        let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
        let flash = FlashStorage::new(peripherals.FLASH);
        provisioning::run_provisioning(usb_serial, flash);
        // Never returns
    }

    let (ssid, password) = {
        #[cfg(feature = "use-env")]
        {
            dprintln!("Using WiFi credentials from .env (use-env feature)");
            (
                alloc::string::String::from(core::env!("SSID")),
                alloc::string::String::from(core::env!("PASSWORD")),
            )
        }
        #[cfg(not(feature = "use-env"))]
        {
            // Check credentials before initializing Embassy/WiFi so FLASH is still free.
            let creds = {
                let flash = FlashStorage::new(peripherals.FLASH);
                get_wifi_credentials(flash) // flash dropped after this block
            };

            match creds {
                Some(c) => c,
                None => {
                    // No credentials stored yet - enter provisioning automatically.
                    dprintln!("No WiFi credentials found - entering setup mode automatically");
                    let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
                    // SAFETY: The FlashStorage created above has been dropped.
                    // Flash operations use ROM functions and hold no exclusive hardware resources,
                    // so stealing the peripheral for a fresh FlashStorage is safe here.
                    let flash = FlashStorage::new(unsafe { esp_hal::peripherals::FLASH::steal() });
                    provisioning::run_provisioning(usb_serial, flash);
                    // Never returns
                }
            }
        }
    };

    // Initialize Embassy executor
    let timg0 = TimerGroup::new(peripherals.TIMG1);
    esp_rtos::start(timg0.timer0);
    dprintln!("Embassy initialized!");

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

    // Read the two-bit board ID from GPIO37 (bit 0) and GPIO38 (bit 1).
    // Both pins are pulled up internally; a resistor to GND grounds a bit.
    // Current board (no resistors) reads 0b11 = Board3.
    let board_id = {
        let gpio37 = gpio::Input::new(
            peripherals.GPIO37,
            gpio::InputConfig::default().with_pull(gpio::Pull::Up),
        );
        let gpio38 = gpio::Input::new(
            peripherals.GPIO38,
            gpio::InputConfig::default().with_pull(gpio::Pull::Up),
        );
        let bits = ((gpio38.is_high() as u8) << 1) | (gpio37.is_high() as u8);
        let id = BoardId::from_bits(bits);
        dprintln!("Board ID: {:?} (0b{:02b})", id, bits);
        id
        // gpio37 and gpio38 are dropped here — GPIO pins freed
    };

    STATUS.lock().await.board_id = board_id;

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

    // USB_DEVICE is free in normal (non-provisioning) boot.
    // debug-serial  → blocking mode, owned by the logger; serial_status not spawned.
    // normal        → async mode, owned by serial_status_task (binary frames).
    #[cfg(feature = "debug-serial")]
    bay_wheels_controller::logger::init(UsbSerialJtag::new(peripherals.USB_DEVICE));

    // Spawn tasks
    dprintln!("Spawning tasks...");
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
        .spawn(input_read::input_read_task(shift_register, board_id))
        .expect("Failed to spawn input_read_task");
    #[cfg(not(feature = "debug-serial"))]
    spawner
        .spawn(serial_status::serial_status_task(
            UsbSerialJtag::new(peripherals.USB_DEVICE).into_async(),
        ))
        .expect("Failed to spawn serial_status_task");
    dprintln!("All tasks spawned!");

    // Main loop - keep alive
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, esp_radio::wifi::WifiDevice<'static>>) {
    runner.run().await
}
