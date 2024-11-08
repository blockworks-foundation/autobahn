use std::collections::HashMap;
use std::env;

use dex_openbook_v2::OpenbookV2Edge;
use router_lib::dex::DexInterface;
use router_lib::test_tools::{generate_dex_rpc_dump, rpc};
use solana_program_test::tokio;

#[tokio::test]
async fn test_dump_input_data_openbook_v2() -> anyhow::Result<()> {
    let options = HashMap::from([]);
    let disable_compressed = std::env::var::<String>("DISABLE_COMRPESSED_GPA".to_string())
        .unwrap_or("false".to_string());
    let disable_compressed: bool = disable_compressed.trim().parse().unwrap();

    if router_test_lib::config_should_dump_mainnet_data() {
        openbook_v2_step_1(&options, !disable_compressed).await?;
    }

    openbook_v2_step_2(&options).await?;

    Ok(())
}

async fn openbook_v2_step_1(
    options: &HashMap<String, String>,
    enable_compression: bool,
) -> anyhow::Result<()> {
    let rpc_url: String = env::var("RPC_HTTP_URL")?;

    let (mut rpc_client, chain_data) =
        rpc::rpc_dumper_client(rpc_url, "openbook_v2_dump.lz4", enable_compression);

    let dex = dex_openbook_v2::OpenbookV2Dex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_mainnet_data_with_custom_amount(
        dex,
        rpc_client,
        chain_data,
        Box::new(|edge| {
            let edge = edge.as_any().downcast_ref::<OpenbookV2Edge>().unwrap();
            5 * edge.market.quote_lot_size.max(edge.market.base_lot_size) as u64
        }),
    )
    .await?;

    Ok(())
}

async fn openbook_v2_step_2(options: &HashMap<String, String>) -> anyhow::Result<()> {
    // Replay
    let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("openbook_v2_dump.lz4");

    let dex = dex_openbook_v2::OpenbookV2Dex::initialize(&mut rpc_client, options.clone()).await?;

    generate_dex_rpc_dump::run_dump_swap_ix_with_custom_amount(
        "openbook_v2_swap.lz4",
        dex,
        chain_data,
        Box::new(|edge| {
            let edge = edge.as_any().downcast_ref::<OpenbookV2Edge>().unwrap();
            5 * edge.market.quote_lot_size.max(edge.market.base_lot_size) as u64
        }),
    )
    .await?;

    Ok(())
}
