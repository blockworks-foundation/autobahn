use itertools::Itertools;
use router_feed_lib::router_rpc_client::RouterRpcClient;
use router_lib::dex::{DexEdgeIdentifier, DexInterface};
use std::collections::HashMap;
use std::sync::Arc;

pub async fn get_all_dex(
    mut rpc_client: &mut RouterRpcClient,
    enable_compression: bool,
) -> anyhow::Result<Vec<Arc<dyn DexInterface>>> {
    let orca_config = HashMap::from([
        (
            "program_id".to_string(),
            "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
        ),
        ("program_name".to_string(), "Orca".to_string()),
    ]);
    let cropper_config = HashMap::from([
        (
            "program_id".to_string(),
            "H8W3ctz92svYg6mkn1UtGfu2aQr2fnUFHM1RhScEtQDt".to_string(),
        ),
        ("program_name".to_string(), "Cropper".to_string()),
    ]);

    let dexs = [
        dex_orca::OrcaDex::initialize(&mut rpc_client, orca_config, enable_compression).await?,
        dex_orca::OrcaDex::initialize(&mut rpc_client, cropper_config, enable_compression).await?,
        dex_saber::SaberDex::initialize(&mut rpc_client, HashMap::new(), enable_compression)
            .await?,
        dex_raydium_cp::RaydiumCpDex::initialize(
            &mut rpc_client,
            HashMap::new(),
            enable_compression,
        )
        .await?,
        dex_raydium::RaydiumDex::initialize(&mut rpc_client, HashMap::new(), enable_compression)
            .await?,
        dex_openbook_v2::OpenbookV2Dex::initialize(
            &mut rpc_client,
            HashMap::new(),
            enable_compression,
        )
        .await?,
        dex_infinity::InfinityDex::initialize(&mut rpc_client, HashMap::new(), enable_compression)
            .await?,
    ];

    Ok(dexs.into_iter().collect())
}

pub fn get_edges_identifiers(dex: &Arc<dyn DexInterface>) -> Vec<Arc<dyn DexEdgeIdentifier>> {
    let edges_identifiers = dex
        .edges_per_pk()
        .into_iter()
        .map(|x| x.1)
        .flatten()
        .unique_by(|x| (x.key(), x.input_mint()))
        .sorted_by_key(|x| (x.key(), x.input_mint()))
        .collect_vec();
    edges_identifiers
}
