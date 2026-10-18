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
    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    static STACK: StaticCell<Stack<'static>> = StaticCell::new();

    let resources = RESOURCES.init(StackResources::new());

    // Use the module-level `new` function to create stack and runner
    let (stack, runner) = embassy_net::new(device, config, resources, 0);

    // Store the stack in a static and return a reference to it
    let stack_ref = STACK.init(stack);

    (stack_ref, runner)
}

pub fn dhcp_config() -> Config {
    let mut dhcp = embassy_net::DhcpConfig::default();
    // smoltcp waits 10s before re-sending an unanswered DISCOVER. The radio is
    // torn down every fetch cycle, so embassy-net restarts DHCP on every
    // reconnect, and the first DISCOVER goes out right as the link comes up and
    // is reliably lost — every cycle's DHCP took ~9.9s, i.e. exactly one retry.
    // Retrying after 1s costs a few tiny broadcasts and saves ~9s of radio-on
    // time per cycle. `DHCP_DISCOVER_MS` at build time overrides the 1s.
    let discover_ms = crate::power::parse_u32(option_env!("DHCP_DISCOVER_MS"), 1000);
    dhcp.retry_config.discover_timeout = smoltcp::time::Duration::from_millis(discover_ms as u64);
    Config::dhcpv4(dhcp)
}
