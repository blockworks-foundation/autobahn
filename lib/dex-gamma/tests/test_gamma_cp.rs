use std::collections::HashMap;
use std::env;

use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_gamma_cp() -> anyhow::Result<()> {
    let options = HashMap::from([]);

    if router_test_lib::config_should_dump_mainnet_data() {
        gamma_cp_step_1(&options).await?;
    }

    gamma_cp_step_2(&options).await?;

    Ok(())
}

async fn gamma_cp_step_1(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;

    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "gamma_dump.lz4");

    let dex = dex_gamma::GammaCpDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn gamma_cp_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    // Replay
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("gamma_dump.lz4");

    let dex = dex_gamma::GammaCpDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("gamma_swap.lz4", dex, chain_data).await?;

    Ok(())
}
