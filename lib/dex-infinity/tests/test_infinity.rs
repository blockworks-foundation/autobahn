use std::collections::HashMap;
use std::env;

use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_infinity() -> anyhow::Result<()> {
    if router_test_lib::config_should_dump_mainnet_data() {
        step_1_infinity().await?;
    }

    step_2_infinity().await?;

    Ok(())
}

async fn step_1_infinity() -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;
    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "infinity_dump.lz4");

    let options = HashMap::from([]);
    let dex = dex_infinity::InfinityDex::initialize(&mut rpc_client, options).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn step_2_infinity() -> anyhow::Result<()> {
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("infinity_dump.lz4");

    let options = HashMap::from([]);
    let dex = dex_infinity::InfinityDex::initialize(&mut rpc_client, options).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("infinity_swap.lz4", dex, chain_data).await?;

    Ok(())
}
