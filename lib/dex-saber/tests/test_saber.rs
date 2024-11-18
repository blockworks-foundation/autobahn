use std::collections::HashMap;
use std::env;

use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_saber() -> anyhow::Result<()> {
    let options = HashMap::from([]);

    if router_test_lib::config_should_dump_mainnet_data() {
        saber_step_1(&options).await?;
    }

    saber_step_2(&options).await?;

    Ok(())
}

async fn saber_step_1(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;
    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "saber_dump.lz4");

    let dex = dex_saber::SaberDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn saber_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("saber_dump.lz4");

    let dex = dex_saber::SaberDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("saber_swap.lz4", dex, chain_data).await?;

    Ok(())
}
