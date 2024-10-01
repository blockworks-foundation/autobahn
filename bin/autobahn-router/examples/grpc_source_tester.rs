use autobahn_router::source::grpc_plugin_source::feed_data_geyser;
use router_config_lib::{AccountDataSourceConfig, GrpcSourceConfig};
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use std::env;
use std::str::FromStr;
use tracing::log::info;

#[tokio::main]
pub async fn main() {
    router_feed_lib::utils::tracing_subscriber_init();

    let grpc_addr = env::var("GRPC_ADDR").expect("need grpc url");
    let grpc_config = GrpcSourceConfig {
        name: "mysource1".to_string(),
        connection_string: grpc_addr.clone(),
        token: None,
        retry_connection_sleep_secs: 3,
        tls: None,
    };

    let rpc_http_addr = env::var("RPC_HTTP_ADDR").expect("need rpc http url");
    let snapshot_config = AccountDataSourceConfig {
        region: None,
        quic_sources: None,
        rpc_http_url: rpc_http_addr.clone(),
        rpc_support_compression: Some(false), /* no compression */
        re_snapshot_interval_secs: None,
        grpc_sources: Some(vec![]),
        dedup_queue_size: 0,
        request_timeout_in_seconds: None,
    };

    // Raydium
    let raydium_program_id =
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap();

    let account_sub = HashSet::new();
    let token_account_sub = HashSet::new();
    let program_sub = HashSet::from([raydium_program_id]);

    let (channel_sender, _dummy_rx) = async_channel::unbounded();

    info!("starting grpc_plugin_source...");
    // blocking
    feed_data_geyser(
        &grpc_config,
        None,
        snapshot_config,
        &account_sub,
        &program_sub,
        &token_account_sub,
        channel_sender,
    )
    .await
    .unwrap();

    info!("DONE.");
}
