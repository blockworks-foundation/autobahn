#[cfg(test)]
mod tests {
    use crate::syscallstubs;
    use crate::tests::dex_test_utils;
    use itertools::Itertools;
    use router_feed_lib::router_rpc_client::RouterRpcClientTrait;
    use router_lib::price_feeds::fillcity::FillCityPriceFeed;
    use router_lib::test_tools::rpc;
    use solana_client::client_error::reqwest::Client;
    use std::collections::HashSet;
    use std::env;
    use std::time::Duration;

    #[tokio::test]
    async fn dump_all_dex_data() -> anyhow::Result<()> {
        if env::var("CI").is_ok() {
            println!("skipping test while running continuous integration");
            return Ok(());
        };

        router_feed_lib::utils::tracing_subscriber_init();
        syscallstubs::deactivate_program_logs();

        let disable_compressed = std::env::var::<String>("DISABLE_COMRPESSED_GPA".to_string())
            .unwrap_or("false".to_string());
        let disable_compressed: bool = disable_compressed.trim().parse().unwrap();

        let rpc_url = env::var("RPC_HTTP_URL")?;
        let (mut rpc_client, _chain_data) =
            rpc::rpc_dumper_client(rpc_url, "all.lz4", !disable_compressed);

        let dexs = dex_test_utils::get_all_dex(&mut rpc_client).await?;

        for dex in &dexs {
            rpc::load_subscriptions(&mut rpc_client, dex.clone()).await?;
        }

        let mut mints = HashSet::new();
        for dex in &dexs {
            let edges_identifiers = dex_test_utils::get_edges_identifiers(&dex);

            for id in edges_identifiers {
                mints.insert(id.input_mint());
                mints.insert(id.output_mint());
            }
        }

        println!("Adding some {} accounts", mints.len());
        rpc_client.get_multiple_accounts(&mints).await?;

        let client = Client::new();
        let mints = mints.into_iter().collect_vec();
        let mut prices = vec![];

        // let mut prices = router_test_lib::serialize::deserialize_from_file::<Vec<PriceUpdate>>(&"all-prices.lz4".to_string())?;
        // let mints = mints.iter().filter(|x| prices.iter().any(|y| y.mint == **x))
        //     .copied()
        //     .collect_vec();
        // println!("Missing prices for {} mints", mints.len());

        for chunk in mints.chunks(150) {
            let res = FillCityPriceFeed::get_prices(&client, &chunk.iter().copied().collect_vec())
                .await?;

            prices.extend(res);
            tokio::time::sleep(Duration::from_millis(700)).await;
        }

        router_test_lib::serialize::serialize_to_file(&prices, &"all-prices.lz4".to_string());

        Ok(())
    }
}
