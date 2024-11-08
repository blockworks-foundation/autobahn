use std::collections::HashMap;
use std::env;

use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_raydium_cp() -> anyhow::Result<()> {
    let options = HashMap::from([]);

    let disable_compressed = std::env::var::<String>("DISABLE_COMRPESSED_GPA".to_string())
        .unwrap_or("false".to_string());
    let disable_compressed: bool = disable_compressed.trim().parse().unwrap();

    if router_test_lib::config_should_dump_mainnet_data() {
        raydium_cp_step_1(&options, !disable_compressed).await?;
    }

    raydium_cp_step_2(&options, !disable_compressed).await?;

    Ok(())
}

async fn raydium_cp_step_1(
    options: &HashMap<String, String>,
    enable_compression: bool,
) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;

    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "raydium_cp_dump.lz4");

    let dex = dex_raydium_cp::RaydiumCpDex::initialize(
        &mut rpc_client,
        options.clone(),
        enable_compression,
    )
    .await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn raydium_cp_step_2(
    options: &HashMap<String, String>,
    enable_compression: bool,
) -> anyhow::Result<()> {
    // Replay
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("raydium_cp_dump.lz4");

    let dex = dex_raydium_cp::RaydiumCpDex::initialize(
        &mut rpc_client,
        options.clone(),
        enable_compression,
    )
    .await?;

    generate_dex_rpc_dump::run_dump_swap_ix("raydium_cp_swap.lz4", dex, chain_data).await?;

    Ok(())
}
