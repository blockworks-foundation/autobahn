use std::collections::HashMap;
use std::env;

use solana_program_test::tokio;

use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};

#[tokio::test]
async fn test_dump_input_data_cropper() -> anyhow::Result<()> {
    let is_eclipse = std::env::var("ECLIPSE")
        .map(|x| {
            let value: bool = x.parse().unwrap();
            value
        })
        .unwrap_or_default();
    if is_eclipse {
        // crooper is not yet on eclipse
        return Ok(());
    }
    let options = HashMap::from([
        (
            "program_id".to_string(),
            "H8W3ctz92svYg6mkn1UtGfu2aQr2fnUFHM1RhScEtQDt".to_string(),
        ),
        ("program_name".to_string(), "Cropper".to_string()),
    ]);

    if router_test_lib::config_should_dump_mainnet_data() {
        cropper_step_1(&options).await?;
    }

    cropper_step_2(&options).await?;

    Ok(())
}

async fn cropper_step_1(options: &HashMap<String, String>) -> anyhow::Result<()> {
    let rpc_url = env::var("RPC_HTTP_URL")?;

    let (mut rpc_client, chain_data) = rpc::rpc_dumper_client(rpc_url, "cropper_dump.lz4");

    let dex = dex_orca::OrcaDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data(dex, rpc_client, chain_data).await?;

    Ok(())
}

async fn cropper_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    // Replay
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("cropper_dump.lz4");

    let dex = dex_orca::OrcaDex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix("cropper_swap.lz4", dex, chain_data).await?;

    Ok(())
}
