use crate::price_feeds::price_feed::{PriceFeed, PriceUpdate};
use anyhow::Context;
use router_config_lib::PriceFeedConfig;
use serde_derive::{Deserialize, Serialize};
use solana_client::client_error::reqwest;
use solana_client::client_error::reqwest::Client;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

pub struct BirdeyeSinglePriceFeed {
    admin_channel_sender: async_channel::Sender<Pubkey>,
    update_sender: broadcast::Sender<PriceUpdate>,
}

impl PriceFeed for BirdeyeSinglePriceFeed {
    fn start(
        configuration: PriceFeedConfig,
        mut exit: Receiver<()>,
    ) -> (impl PriceFeed, JoinHandle<()>) {
        let (admin_channel_sender, admin_channel_receiver) = async_channel::unbounded::<Pubkey>();
        let (update_sender, _) = broadcast::channel::<PriceUpdate>(1000);
        let refresh_interval = Duration::from_secs(configuration.refresh_interval_secs);
        let token = configuration.birdeye_token.clone();

        let update_sender_clone = update_sender.clone();
        let join_handle = tokio::spawn(async move {
            let mut mints = HashSet::<Pubkey>::new();
            let mut interval = tokio::time::interval(refresh_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = exit.recv() => {
                        info!("Exit signal received, stopping price feed..");
                        break;
                    },
                    Ok(new_mint) = admin_channel_receiver.recv() => {
                        if mints.insert(new_mint) {
                            debug!("Adding {} to price feed subscriptions", new_mint);
                            interval.reset_after(Duration::from_millis(100));
                        }
                    },
                    _ = interval.tick() => {
                        debug!("Refreshing price for {} mint(s)", mints.len());

                        let res = BirdeyeSinglePriceFeed::refresh(token.clone(), &mints, update_sender_clone.clone()).await;
                        if res.is_err() {
                            error!("Price feed error: {}", res.unwrap_err())
                        }
                    }
                }
            }

            info!("price feed exited")
        });

        let feed = BirdeyeSinglePriceFeed {
            admin_channel_sender,
            update_sender,
        };

        (feed, join_handle)
    }

    fn receiver(&mut self) -> Receiver<PriceUpdate> {
        self.update_sender.subscribe()
    }

    fn register_mint_sender(&self) -> async_channel::Sender<Pubkey> {
        self.admin_channel_sender.clone()
    }
}

impl BirdeyeSinglePriceFeed {
    pub async fn refresh(
        api_token: String,
        mints: &HashSet<Pubkey>,
        sender: broadcast::Sender<PriceUpdate>,
    ) -> anyhow::Result<()> {
        let http_client = reqwest::Client::new();

        for address in mints {
            let Ok(Some(price)) = Self::get_one_price(&api_token, &http_client, address).await
            else {
                continue;
            };

            let res = sender.send(price);
            debug!(
                " - price updated to {} receivers for {} mints",
                res.ok().unwrap_or(0),
                mints.len()
            );
        }

        Ok(())
    }

    pub async fn get_one_price(
        api_token: &str,
        http_client: &Client,
        address: &Pubkey,
    ) -> anyhow::Result<Option<PriceUpdate>> {
        let query_args = vec![("address", address.to_string())];
        let response = http_client
            .get("https://public-api.birdeye.so/defi/price")
            .query(&query_args)
            .header("X-API-KEY", api_token)
            .header("Origin", "https://autobahn.mngo.cloud")
            .send()
            .await
            .context("birdeye (single) request")?;

        let prices: anyhow::Result<BirdEyePricesResponse> =
            crate::utils::http_error_handling(response).await;

        let prices = match prices {
            Ok(r) => r,
            Err(e) => {
                anyhow::bail!("error requesting birdeye (single) prices: {}", e);
            }
        };

        if let Some(price) = prices.data {
            debug!(" - price updated for {} -> {}", address, price.value);
            return Ok(Some(PriceUpdate {
                mint: *address,
                price: price.value,
            }));
        }

        debug!(" - no price for {}", address);
        Ok(None)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BirdEyePricesResponse {
    pub data: Option<BirdEyePrice>,
    pub success: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BirdEyePrice {
    pub value: f64,
}

#[cfg(test)]
mod tests {
    use crate::price_feeds::birdeye_single::BirdeyeSinglePriceFeed;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[ignore]
    #[tokio::test]
    pub async fn should_fetch_one_price() -> anyhow::Result<()> {
        let http_client = reqwest::Client::new();
        let pubkey = Pubkey::from_str("EAyTvjaCGq2JYwuAPQWcFMua7FBJnfhVGJgcF3QZ2fAJ").unwrap();
        let token = "<TOKEN>";
        let result = BirdeyeSinglePriceFeed::get_one_price(token, &http_client, &pubkey).await?;
        println!("{}", result.unwrap().price);
        todo!()
    }
}
