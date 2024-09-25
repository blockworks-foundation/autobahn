use itertools::Itertools;
use router_lib::dex::SwapMode;
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tracing::log::trace;
use tracing::{debug, info, warn};

use crate::debug_tools;
use crate::hot_mints::HotMintsCache;
use crate::server::route_provider::RouteProvider;
use crate::token_cache::TokenCache;
use router_config_lib::{PathWarmingMode, RoutingConfig};
use router_lib::mango::mango_fetcher::MangoMetadata;
use router_lib::price_feeds::price_cache::PriceCache;

pub fn spawn_path_warmer_job<T>(
    config: &RoutingConfig,
    hot_mints_cache: Arc<RwLock<HotMintsCache>>,
    mango_metadata: Option<MangoMetadata>,
    route_provider: Arc<T>,
    token_cache: TokenCache,
    price_cache: PriceCache,
    path_warming_amounts: Vec<u64>,
    exit_flag: Arc<AtomicBool>,
) -> Option<JoinHandle<()>>
where
    T: RouteProvider + Send + Sync + 'static,
{
    let mode = config
        .path_warming_mode
        .clone()
        .unwrap_or(PathWarmingMode::ConfiguredMints);
    let configured_mints = config
        .path_warming_for_mints
        .clone()
        .unwrap_or(vec![])
        .iter()
        .map(|x| Pubkey::from_str(x).expect("Invalid mint in path warming config"))
        .collect_vec();

    match mode {
        PathWarmingMode::None => return None,
        PathWarmingMode::ConfiguredMints => {
            if configured_mints.is_empty() {
                warn!("No configured tokens => no path warming");
                return None;
            }
        }
        PathWarmingMode::MangoMints => {
            if mango_metadata.is_none() {
                warn!("Mango tokens unavailable => no path warming");
                return None;
            }
        }
        PathWarmingMode::HotMints => {}
        PathWarmingMode::All => {}
    };

    let sol_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
    let config = config.clone();
    let start = Instant::now();
    let job = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            config.path_warming_interval_secs.unwrap_or(10),
        ));
        let config_max_accounts = config
            .path_warming_max_accounts
            .unwrap_or(vec![10_usize, 15, 20, 25, 30, 40]);
        interval.tick().await;

        loop {
            interval.tick().await;
            if start.elapsed() < Duration::from_secs(60) {
                // do not start right away as not everything is ready yet
                continue;
            }

            let mut all_mints = token_cache.tokens();
            all_mints.insert(sol_mint);

            let hot_mints = hot_mints_cache.read().unwrap().get();
            let mints = match generate_mints(
                &mode,
                &configured_mints,
                &hot_mints,
                &all_mints,
                &mango_metadata,
            ) {
                Some(value) => value,
                None => break,
            };

            debug!("Running a path warmup loop for {} mints", mints.len());
            let mut counter = 0;
            let mut skipped = 0;
            let time = Instant::now();

            // prune edges for exact in
            route_provider.prepare_pruned_edges_and_cleanup_cache(&hot_mints, SwapMode::ExactIn);

            // prune edges for exact out
            route_provider.prepare_pruned_edges_and_cleanup_cache(&hot_mints, SwapMode::ExactOut);

            for from_mint in &mints {
                if exit_flag.load(Ordering::Relaxed) {
                    tracing::log::warn!("shutting down path warmer job...");
                    return ();
                }

                let Some(price_ui) = price_cache.price_ui(*from_mint) else {
                    skipped += 1;
                    continue;
                };
                if price_ui <= 0.000001 {
                    skipped += 1;
                    continue;
                }
                let Ok(token) = token_cache.token(*from_mint) else {
                    skipped += 1;
                    continue;
                };

                let decimals = token.decimals;
                let multiplier = 10u64.pow(decimals as u32) as f64;

                trace!("Warming up {}", debug_tools::name(&from_mint),);

                for amount_ui in &path_warming_amounts {
                    let amount_native =
                        ((*amount_ui as f64 / price_ui) * multiplier).round() as u64;

                    for max_accounts in &config_max_accounts {
                        match route_provider.prepare_cache_for_input_mint(
                            *from_mint,
                            amount_native,
                            *max_accounts,
                            |input, output| mints.contains(input) || mints.contains(output),
                        ) {
                            Ok(_) => {}
                            Err(e) => warn!("Error warming up path: {}", e),
                        };
                    }
                }

                counter += 1;

                if counter % 100 == 0 {
                    debug!(
                        "Done for {}/{} mints (skipped {})",
                        counter,
                        mints.len(),
                        skipped
                    );
                }
            }

            info!(
                "Path warmup done in {:?} for {} mints",
                time.elapsed(),
                mints.len()
            );
        }
    });

    Some(job)
}

fn generate_mints(
    mode: &PathWarmingMode,
    configured_mints: &Vec<Pubkey>,
    hot_mints: &HashSet<Pubkey>,
    all_mints: &HashSet<Pubkey>,
    mango_metadata: &Option<MangoMetadata>,
) -> Option<HashSet<Pubkey>> {
    Some(match mode {
        PathWarmingMode::None => return None,
        PathWarmingMode::ConfiguredMints => configured_mints.clone().into_iter().collect(),
        PathWarmingMode::HotMints => hot_mints.clone().into_iter().collect(),
        PathWarmingMode::MangoMints => mango_metadata.as_ref().unwrap().mints.clone(),
        PathWarmingMode::All => all_mints.clone(),
    })
}
