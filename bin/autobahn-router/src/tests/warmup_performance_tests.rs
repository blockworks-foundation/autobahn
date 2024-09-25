#[cfg(test)]
mod tests {
    use crate::routing::Routing;
    use crate::syscallstubs;
    use crate::tests::dex_test_utils;
    use itertools::Itertools;
    use rand::random;
    use router_config_lib::Config;
    use router_lib::dex::{AccountProviderView, ChainDataAccountProvider, SwapMode};
    use router_lib::test_tools::rpc;
    use solana_program::pubkey::Pubkey;
    use std::collections::HashSet;
    use std::env;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Instant;

    #[tokio::test]
    async fn path_warmup_perf_test() -> anyhow::Result<()> {
        if env::var("CI").is_ok() {
            println!("skipping test while running continuous integration");
            return Ok(());
        };

        syscallstubs::deactivate_program_logs();

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
        let pwa = vec![100];
        // let pwa = vec![100, 1_000, 10_000];

        for edge in &edges {
            edge.update_internal(
                &chain_data,
                6,                     // TODO FAS Real decimals
                1.0 + random::<f64>(), // TODO FAS Real price
                &pwa,
            );
        }

        let mut config = Config::default();
        config.routing.path_cache_validity_ms = 500 * 60 * 1_000; // berk
        config.routing.max_path_length = Some(3);
        config.routing.retain_path_count = Some(4);
        config.routing.max_edge_per_pair = Some(4);
        let routing = Routing::new(&config, pwa.clone(), edges.clone());
        let mints = edges.iter().map(|x| x.input_mint).collect::<HashSet<_>>();
        let configured_mints = mints
            .iter()
            .take(100)
            .map(|x| *x)
            .chain(vec![
                Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(),
                Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            ])
            .collect::<HashSet<_>>();

        let start = Instant::now();
        let mut counter = 0;

        routing.prepare_pruned_edges_and_cleanup_cache(&configured_mints, SwapMode::ExactIn);

        println!("number of mints: {}", mints.len());
        println!("number of configured_mints: {}", configured_mints.len());
        println!("number of edges: {}", edges.len());

        for m in &configured_mints {
            for amount_ui in &pwa {
                for max_accounts in [40] {
                    // for max_accounts in [10, 15, 20, 25, 30, 40] {
                    let _ = routing.prepare_cache_for_input_mint(
                        m,
                        *amount_ui,
                        max_accounts,
                        |i, o| configured_mints.contains(i) || configured_mints.contains(o),
                    );
                }
            }

            counter += 1;
            if counter % 100 == 0 {
                println!("-> {} in {:?}", counter, start.elapsed())
            }
        }

        println!(
            "duration: {}ms",
            start.elapsed().as_micros() as f64 / 1000.0
        );

        for _i in 0..3 {
            bench_one_path_resolve(&chain_data, &routing);
        }

        Ok(())
    }

    fn bench_one_path_resolve(chain_data: &AccountProviderView, routing: &Routing) {
        let start = Instant::now();
        let path = routing
            .find_best_route(
                &chain_data,
                &Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(),
                &Pubkey::from_str("J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn").unwrap(),
                50_000_000,
                40,
                false,
                &HashSet::new(),
                None,
                SwapMode::ExactIn,
            )
            .unwrap();

        println!(
            "duration: {}ms",
            start.elapsed().as_micros() as f64 / 1000.0
        );
        println!("out_amount: {}", path.out_amount);
        println!("price_impact (bps): {}", path.price_impact_bps);
        println!("steps count: {}", path.steps.len());
    }
}
