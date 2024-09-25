///
/// Replaces this:
/// ```
/// router_test_lib::config_should_dump_mainnet_data()
/// ```
pub fn config_should_dump_mainnet_data() -> bool {
    match std::env::var("DUMP_MAINNET_DATA") {
        Ok(val) => val != "0",
        Err(_) => false,
    }
}
