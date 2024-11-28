use std::collections::HashMap;
use std::env;

use router_feed_lib::utils::tracing_subscriber_init;
use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_orca() -> anyhow::Result<()> {
    tracing_subscriber_init();
    let options = HashMap::from([
        ("program_id".to_string(), whirlpools_client::ID.to_string()),
        ("program_name".to_string(), "Orca".to_string()),
    ]);
    if router_test_lib::config_should_dump_mainnet_data() {
        orca_step_1(&options).await?;
    }

    orca_step_2(&options).await?;

    Ok(())
}

async fn orca_step_1(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;
    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "orca_dump.lz4");

    let dex = dex_orca::OrcaDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn orca_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("orca_dump.lz4");

    let dex = dex_orca::OrcaDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("orca_swap.lz4", dex, chain_data).await?;

    Ok(())
}
