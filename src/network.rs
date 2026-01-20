use embassy_net::{Config, Runner, Stack, StackResources};
use esp_radio::wifi::WifiDevice;
use static_cell::StaticCell;

pub fn create_network_stack(
    device: WifiDevice<'static>,
    config: Config,
) -> (
    &'static Stack<'static>,
    Runner<'static, WifiDevice<'static>>,
) {
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    static STACK: StaticCell<Stack<'static>> = StaticCell::new();

    let resources = RESOURCES.init(StackResources::new());

    // Use the module-level `new` function to create stack and runner
    let (stack, runner) = embassy_net::new(device, config, resources, 0);

    // Store the stack in a static and return a reference to it
    let stack_ref = STACK.init(stack);

    (stack_ref, runner)
}

pub fn dhcp_config() -> Config {
    Config::dhcpv4(Default::default())
}
