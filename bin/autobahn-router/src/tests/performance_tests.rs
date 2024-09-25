#[cfg(test)]
mod tests {
    use crate::edge::Edge;
    use crate::routing::Routing;
    use crate::tests::dex_test_utils;
    use crate::{debug_tools, syscallstubs};
    use anchor_spl::token::spl_token::state::Mint;
    use itertools::{iproduct, Itertools};
    use router_config_lib::Config;
    use router_lib::dex::{AccountProviderView, ChainDataAccountProvider, SwapMode};
    use router_lib::price_feeds::price_feed::PriceUpdate;
    use router_lib::test_tools::rpc;
    use solana_program::program_pack::Pack;
    use solana_program::pubkey::Pubkey;
    use solana_sdk::account::ReadableAccount;
    use std::collections::{HashMap, HashSet};
    use std::env;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Instant;
    use tracing::{info, warn};

    #[tokio::test]
    async fn path_finding_perf_test() -> anyhow::Result<()> {
        if env::var("CI").is_ok() {
            println!("skipping test while running continuous integration");
            return Ok(());
        };

        router_feed_lib::utils::tracing_subscriber_init();
        syscallstubs::deactivate_program_logs();

        let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let _jupsol = Pubkey::from_str("jupSoLaHXQiZZTSfEWMTRRgpnyFm8f6sZdosWBjx93v").unwrap();
        let _vsol = Pubkey::from_str("vSoLxydx6akxyMD9XEcPvGYNGq6Nn66oqVb3UkGkei7").unwrap();
        let mnde = Pubkey::from_str("MNDEFzGvMt87ueuHvVU9VcTqsAP5b3fTGPsHuuPA5ey").unwrap();

        let (mut rpc_client, chain_data) = rpc::rpc_replayer_client("all.lz4");
        let chain_data = Arc::new(ChainDataAccountProvider::new(chain_data)) as AccountProviderView;
        let dex_sources = dex_test_utils::get_all_dex(&mut rpc_client).await?;
        let mut dexs = vec![];
        for dex in dex_sources {
            dexs.push(
                crate::dex::generic::build_dex_internal(dex, &None, true, false, true, &vec![])
                    .await?,
            );
        }
        let edges = dexs.iter().map(|x| x.edges()).flatten().collect_vec();

        let mut config = Config::default();
        config.routing.path_cache_validity_ms = 0;
        config.routing.max_path_length = Some(3);
        config.routing.retain_path_count = Some(5);
        config.routing.max_edge_per_pair = Some(8);
        config.routing.max_edge_per_cold_pair = Some(3);
        let pwa = vec![100, 1_000, 10_000];

        let prices = router_test_lib::serialize::deserialize_from_file::<Vec<PriceUpdate>>(
            &"all-prices.lz4".to_string(),
        )?
        .into_iter()
        .map(|x| (x.mint, x.price))
        .collect::<HashMap<Pubkey, f64>>();

        for edge in &edges {
            let decimals = {
                let Ok(mint_account) = chain_data.account(&edge.input_mint) else {
                    warn!("Missing mint {}", edge.input_mint);
                    continue;
                };
                let mint = Mint::unpack(mint_account.account.data())?;
                mint.decimals
            };
            edge.update_internal(
                &chain_data,
                decimals,
                *prices.get(&edge.input_mint).unwrap_or(&0.0),
                &pwa,
            );
        }

        let _mints = edges.iter().map(|x| x.input_mint).collect::<HashSet<_>>();
        let available_mints = get_reachable_mints(usdc, sol, &edges);

        let hot_mints = available_mints
            .iter()
            .take(100)
            .map(|x| *x)
            .chain(vec![
                usdc,
                mnde,
                Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            ])
            .collect::<HashSet<_>>();

        let routing = Routing::new(&config, pwa, edges);
        routing.prepare_pruned_edges_and_cleanup_cache(&hot_mints, SwapMode::ExactIn);

        let input_mints = [usdc, sol];
        let sorted_mints = available_mints.iter().sorted();
        // let sorted_mints = &[jupsol, mnde, vsol];

        let pairs = iproduct!(input_mints.iter(), sorted_mints.into_iter())
            .filter(|x| *x.0 != *x.1)
            .map(|x| (*x.0, *x.1))
            .collect::<Vec<(Pubkey, Pubkey)>>();

        let mut failures = HashSet::new();

        for p in pairs {
            for max_account in [30, 40, 64] {
                for amounts in [50_000_000, 300_000_000, 2_000_000_000] {
                    let start = Instant::now();
                    let path = routing.find_best_route(
                        &chain_data,
                        &p.0,
                        &p.1,
                        amounts,
                        max_account,
                        false,
                        &hot_mints,
                        None,
                        SwapMode::ExactIn,
                    );

                    match path {
                        Ok(path) => {
                            let in_amount_dollars =
                                get_price(&chain_data, &prices, &p.0, path.in_amount)?;
                            let out_amount_dollars =
                                get_price(&chain_data, &prices, &p.1, path.out_amount)?;

                            if (out_amount_dollars as f64) < (in_amount_dollars as f64) * 0.7 {
                                warn!(
                                    "{} -> {} in {}ms ({} hop(s)) [{}$ -> {}$]",
                                    debug_tools::name(&p.0),
                                    debug_tools::name(&p.1),
                                    start.elapsed().as_micros() as f64 / 1000.0,
                                    path.steps.len(),
                                    in_amount_dollars,
                                    out_amount_dollars,
                                );
                            } else {
                                info!(
                                    "{} -> {} in {}ms ({} hop(s)) [{}$ -> {}$]",
                                    debug_tools::name(&p.0),
                                    debug_tools::name(&p.1),
                                    start.elapsed().as_micros() as f64 / 1000.0,
                                    path.steps.len(),
                                    in_amount_dollars,
                                    out_amount_dollars,
                                );
                            }
                            // println!("price_impact (bps): {}", path.price_impact_bps);
                        }
                        Err(_err) => {
                            failures.insert(p);
                        }
                    }
                }
            }
        }

        for (f, t) in failures {
            warn!("Quote failed for {} -> {}", f, t)
        }

        Ok(())
    }

    fn get_price(
        chain_data: &AccountProviderView,
        prices: &HashMap<Pubkey, f64>,
        key: &Pubkey,
        amount: u64,
    ) -> anyhow::Result<u64> {
        let decimals = {
            let mint_account = chain_data.account(key)?;
            let mint = Mint::unpack(mint_account.account.data())?;
            mint.decimals
        };

        let p = *prices.get(key).unwrap_or(&0.0);
        let d = 10_u64.pow(decimals as u32) as f64;
        let amount_ui = (p * amount as f64).div_euclid(d);

        Ok(amount_ui.floor() as u64)
    }

    fn get_reachable_mints(usdc: Pubkey, sol: Pubkey, edges: &Vec<Arc<Edge>>) -> HashSet<Pubkey> {
        let mut available_mints = HashSet::from([usdc, sol]);
        loop {
            let mut any_new = false;

            for edge in edges {
                if !edge.state.read().unwrap().is_valid() {
                    continue;
                }

                if available_mints.contains(&edge.input_mint) {
                    if available_mints.insert(edge.output_mint) {
                        any_new = true;
                    }
                }
            }

            if any_new == false {
                break;
            }
        }
        available_mints
    }
}
