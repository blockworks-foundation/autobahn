use crate::debug_tools;
use crate::edge::Edge;
use crate::liquidity::liquidity_computer::compute_liquidity;
use crate::liquidity::liquidity_provider::LiquidityProviderArcRw;
use crate::util::tokio_spawn;
use itertools::Itertools;
use router_lib::dex::AccountProviderView;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub fn spawn_liquidity_updater_job(
    provider: LiquidityProviderArcRw,
    edges: Vec<Arc<Edge>>,
    chain_data: AccountProviderView,
    mut exit: broadcast::Receiver<()>,
) -> JoinHandle<()> {
    let job = tokio_spawn("liquidity_updater", async move {
        let mut refresh_all_interval = tokio::time::interval(Duration::from_secs(30));
        refresh_all_interval.tick().await;

        loop {
            tokio::select! {
                _ = exit.recv() => {
                    info!("shutting down liquidity_updater task");
                    break;
                }
                _ = refresh_all_interval.tick() => {
                    refresh_liquidity(&provider, &edges, &chain_data);
                }
            }
        }
    });

    job
}

fn refresh_liquidity(
    provider: &LiquidityProviderArcRw,
    edges: &Vec<Arc<Edge>>,
    account_provider: &AccountProviderView,
) {
    for edge in edges {
        let liquidity = compute_liquidity(&edge, &account_provider);
        if let Ok(liquidity) = liquidity {
            provider
                .write()
                .unwrap()
                .set_liquidity(edge.output_mint, edge.id.key(), liquidity);
        } else {
            debug!("Could not compute liquidity for {}", edge.id.desc())
        }
    }

    for mint in edges.iter().map(|x| x.output_mint).unique() {
        debug!(
            "Liquidity for {} -> {}",
            debug_tools::name(&mint),
            provider.read().unwrap().get_total_liquidity_native(mint)
        )
    }
}
