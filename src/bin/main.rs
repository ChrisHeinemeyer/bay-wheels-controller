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
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{clock::CpuClock, gpio, timer::timg::TimerGroup};
use rtt_target::rprintln;

use esp32_embasssy_wifi_test::tasks::{blink, fetch, wifi_connect};
use esp32_embasssy_wifi_test::{network, wifi};

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Initialize RTT for logging
    rtt_target::rtt_init_print!();
    rprintln!("Starting ESP32-C6...");

    // Initialize ESP-HAL with max CPU clock
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Setup heap allocator
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);

    // Initialize Embassy executor
    let timg0 = TimerGroup::new(peripherals.TIMG1);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    rprintln!("Embassy initialized!");

    // Initialize WiFi radio
    let radio_init_value = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    let radio_init: &'static _ = Box::leak(Box::new(radio_init_value));
    let (controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    // WiFi credentials
    static SSID: &str = env!("SSID");
    static PASSWORD: &str = env!("PASSWORD");

    // Spawn WiFi connection task
    spawner
        .spawn(wifi_connect::wifi_connect_task(controller, SSID, PASSWORD))
        .expect("Failed to spawn wifi_connect_task");

    // Setup embassy-net stack for networking
    let (stack, runner) = network::create_network_stack(interfaces.sta, network::dhcp_config());

    // Spawn network runner task to handle the stack
    spawner
        .spawn(net_task(runner))
        .expect("Failed to spawn net_task");

    // Setup WiFi sniffer
    wifi::setup_sniffer(interfaces.sniffer);
    rprintln!("WiFi sniffer configured!");

    // Setup LED on GPIO15
    let led_pin = gpio::Output::new(
        peripherals.GPIO15,
        gpio::Level::Low,
        gpio::OutputConfig::default(),
    );

    // Spawn tasks
    rprintln!("Spawning tasks...");
    spawner
        .spawn(blink::blink_task(led_pin))
        .expect("Failed to spawn blink_task");
    spawner
        .spawn(fetch::fetch_task(stack))
        .expect("Failed to spawn fetch_task");
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
