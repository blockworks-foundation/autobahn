use router_feed_lib::utils::tracing_subscriber_init;
use solana_program_test::tokio;
use std::collections::HashMap;
use std::env;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_raydium() -> anyhow::Result<()> {
    tracing_subscriber_init();
    let options = HashMap::from([]);

    if router_test_lib::config_should_dump_mainnet_data() {
        raydium_step_1(&options).await?;
    }

    raydium_step_2(&options).await?;

    Ok(())
}

async fn raydium_step_1(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;
    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "raydium_dump.lz4");

    let dex = dex_raydium::RaydiumDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn raydium_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("raydium_dump.lz4");

    let dex = dex_raydium::RaydiumDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("raydium_swap.lz4", dex, chain_data).await?;

    Ok(())
}
