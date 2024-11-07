use std::collections::HashMap;
use std::sync::Arc;

use itertools::Itertools;
use router_lib::dex::{DexInterface, DexSubscriptionMode};
use router_lib::mango::mango_fetcher::MangoMetadata;
use tracing::{info, trace};

use crate::edge::Edge;
use crate::edge_updater::Dex;
use crate::utils;

#[macro_export]
macro_rules! build_dex {
    ($dex_builder:expr, $metadata:expr, $enabled:expr, $add_mango_tokens:expr, $take_all_mints:expr, $mints:expr) => {
        if $enabled {
            let dex = $dex_builder;
            let result = crate::dex::generic::build_dex_internal(
                dex,
                $metadata,
                $enabled,
                $add_mango_tokens,
                $take_all_mints,
                $mints,
            )
            .await?;
            Some(result)
        } else {
            None
        }
    };
}

pub(crate) use build_dex;

pub async fn build_dex_internal(
    dex: Arc<dyn DexInterface>,
    mango_metadata: &Option<MangoMetadata>,
    enabled: bool,
    add_mango_tokens: bool,
    take_all_mints: bool,
    mints: &Vec<String>,
) -> anyhow::Result<Dex> {
    let mints = utils::get_configured_mints(&mango_metadata, enabled, add_mango_tokens, mints)?;

    let edges_per_pk_src = dex.edges_per_pk();
    let mut edges_per_pk = HashMap::new();

    info!("dex {} enabled={enabled} add_mango_tokens={add_mango_tokens} take_all_mints={take_all_mints} mints={mints:?} edges={}", dex.name(), edges_per_pk_src.len());

    for (key, edge_ids) in edges_per_pk_src {

        let edges = edge_ids.clone()
            .into_iter()
            .filter(|x| {
                let keep = take_all_mints
                    || (mints.contains(&x.input_mint()) && mints.contains(&x.output_mint()));
                keep
            })
            .map(|x| {
                Arc::new(Edge {
                    input_mint: x.input_mint(),
                    output_mint: x.output_mint(),
                    accounts_needed: x.accounts_needed(),
                    dex: dex.clone(),
                    id: x,
                    state: Default::default(),
                })
            })
            .collect_vec();

        trace!("build_dex_internal key={key:?} edge_ids={} edges={}", edge_ids.len(), edges.len());

        if edges.len() > 0 {
            edges_per_pk.insert(key, edges);
        }
    }

    let subscription_mode = match dex.subscription_mode() {
        DexSubscriptionMode::Disabled => DexSubscriptionMode::Disabled,
        DexSubscriptionMode::Accounts(a) => {
            if take_all_mints {
                DexSubscriptionMode::Accounts(a)
            } else {
                DexSubscriptionMode::Accounts(edges_per_pk.keys().map(|x| x.clone()).collect())
            }
        }
        DexSubscriptionMode::Programs(p) => DexSubscriptionMode::Programs(p),
        DexSubscriptionMode::Mixed(m) => DexSubscriptionMode::Mixed(m),
    };

    info!("Dex {} will subscribe to {}", dex.name(), subscription_mode);

    Ok(Dex {
        name: dex.name(),
        edges_per_pk,
        subscription_mode: if enabled {
            subscription_mode
        } else {
            DexSubscriptionMode::Disabled
        },
    })
}
