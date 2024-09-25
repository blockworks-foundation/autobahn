use crate::fail_or_retry;
use crate::price_feeds::price_feed::{PriceFeed, PriceUpdate};
use crate::retry_counter::RetryCounter;
use anyhow::Context;
use itertools::Itertools;
use reqwest::Client;
use router_config_lib::PriceFeedConfig;
use serde_derive::{Deserialize, Serialize};
use solana_client::client_error::reqwest;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// C&P variant of BirdeyePriceFeed
pub struct FillCityPriceFeed {
    admin_channel_sender: async_channel::Sender<Pubkey>,
    update_sender: broadcast::Sender<PriceUpdate>,
}

impl PriceFeed for FillCityPriceFeed {
    fn start(
        configuration: PriceFeedConfig,
        mut exit: Receiver<()>,
    ) -> (impl PriceFeed, JoinHandle<()>) {
        info!("Starting fillcity price feed..");
        let (admin_channel_sender, admin_channel_receiver) = async_channel::unbounded::<Pubkey>();
        let (update_sender, _) = broadcast::channel::<PriceUpdate>(1000);
        let refresh_interval = Duration::from_secs(configuration.refresh_interval_secs);

        let update_sender_clone = update_sender.clone();
        let join_handle = tokio::spawn(async move {
            let mut mints = HashSet::<Pubkey>::new();
            let mut interval = tokio::time::interval(refresh_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = exit.recv() => {
                        info!("Exit signal received, stopping fillcity price feed..");
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

                        let res = FillCityPriceFeed::refresh(&mints, update_sender_clone.clone()).await;
                        if res.is_err() {
                            error!("Price feed error: {}", res.unwrap_err())
                        }
                    }
                }
            }

            info!("fillcity price feed exited")
        });

        let feed = FillCityPriceFeed {
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

impl FillCityPriceFeed {
    pub async fn refresh(
        mints: &HashSet<Pubkey>,
        sender: broadcast::Sender<PriceUpdate>,
    ) -> anyhow::Result<()> {
        let http_client = reqwest::Client::new();

        let mut chunks: Vec<Vec<Pubkey>> = vec![];
        for chunk in &mints.iter().chunks(50) {
            chunks.push(chunk.copied().collect());
        }

        for chunk in chunks {
            let mut retry_counter = RetryCounter::new(3, Duration::from_millis(750));
            let result =
                fail_or_retry!(retry_counter, Self::get_prices(&http_client, &chunk).await)
                    .unwrap_or(vec![]);

            for res in result {
                if sender.receiver_count() > 0 {
                    sender.send(res)?;
                }
            }
        }

        Ok(())
    }

    pub async fn get_prices(
        http_client: &Client,
        chunk: &[Pubkey],
    ) -> anyhow::Result<Vec<PriceUpdate>> {
        let address = chunk.iter().map(|x| x.to_string()).join(",");
        let query_args = vec![("mints", address)];
        let response = http_client
            .get("https://api.mngo.cloud/traffic/v1/last-price")
            .query(&query_args)
            .send()
            .await
            .context("fillcity prices request")?;

        let prices: anyhow::Result<FillCityPricesResponse> =
            crate::utils::http_error_handling(response).await;

        let prices = match prices {
            Ok(r) => r,
            Err(e) => {
                error!("error requesting fillcity prices: {}", e);
                return Err(e);
            }
        };

        let mut result = vec![];
        for item in prices.data.into_iter().flatten() {
            if let Some(price) = item.price {
                let res = PriceUpdate {
                    mint: Pubkey::from_str(&item.mint).unwrap(),
                    price,
                };
                result.push(res);
                debug!(" - price updated for {} -> {}", item.mint, price);
            } else {
                debug!(" - no price for {}", item.mint);
            }
        }

        // fillcity give price is usdc, but for what we are doing, it's fine to consider it's usd, and hardcode usdc to 1.0
        result.push(PriceUpdate {
            mint: Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(),
            price: 1.0,
        });
        Ok(result)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct FillCityPricesResponse {
    pub data: Vec<Option<FillCityPrice>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct FillCityPrice {
    pub mint: String,
    pub price: Option<f64>,
}
