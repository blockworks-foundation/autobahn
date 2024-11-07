use crate::price_feeds::price_feed::{PriceFeed, PriceUpdate};
use anyhow::Context;
use itertools::Itertools;
use router_config_lib::PriceFeedConfig;
use serde_derive::{Deserialize, Serialize};
use solana_client::client_error::reqwest;
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

pub struct BirdeyePriceFeed {
    admin_channel_sender: async_channel::Sender<Pubkey>,
    update_sender: broadcast::Sender<PriceUpdate>,
}

impl PriceFeed for BirdeyePriceFeed {
    fn start(
        configuration: PriceFeedConfig,
        mut exit: Receiver<()>,
    ) -> (impl PriceFeed, JoinHandle<()>) {
        // note: this requires a paid API key from birdeye
        info!("Starting birdeye price feed..");
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
                        info!("Exit signal received, stopping birdeye price feed..");
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

                        let res = BirdeyePriceFeed::refresh(token.clone(), &mints, update_sender_clone.clone()).await;
                        if res.is_err() {
                            error!("Price feed error: {}", res.unwrap_err())
                        }
                    }
                }
            }

            info!("birdeye price feed exited")
        });

        let feed = BirdeyePriceFeed {
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

impl BirdeyePriceFeed {
    pub async fn refresh(
        api_token: String,
        mints: &HashSet<Pubkey>,
        sender: broadcast::Sender<PriceUpdate>,
    ) -> anyhow::Result<()> {
        return Ok(());
        let http_client = reqwest::Client::new();

        let mut chunks: Vec<Vec<Pubkey>> = vec![];
        for chunk in &mints.iter().chunks(50) {
            chunks.push(chunk.copied().collect());
        }

        'chunk_loop: for chunk in chunks {
            let address = chunk.iter().map(|x| x.to_string()).join(",");
            let query_args = vec![("list_address", address)];
            let response = http_client
                .get("https://public-api.birdeye.so/defi/multi_price")
                .query(&query_args)
                .header("X-API-KEY", api_token.clone())
                .header("Origin", "https://autobahn.mngo.cloud")
                .send()
                .await
                .context("birdeye request")?;

            let prices: anyhow::Result<BirdEyePricesResponse> =
                crate::utils::http_error_handling(response).await;

            let prices = match prices {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        "error requesting birdeye prices for chunk: {} - continue with next chunk",
                        e
                    );
                    continue 'chunk_loop;
                }
            };

            for (mint, price) in prices.data {
                if let Some(price) = price {
                    let res = sender.send(PriceUpdate {
                        mint: Pubkey::from_str(&mint).unwrap(),
                        price: price.value,
                    });
                    debug!(
                        " - price updated to {} receivers for {} -> {}",
                        res.ok().unwrap_or(0),
                        mint,
                        price.value
                    );
                } else {
                    debug!(" - no price for {}", mint);
                }
            }
        }

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BirdEyePricesResponse {
    pub data: HashMap<String, Option<BirdEyePrice>>,
    pub success: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BirdEyePrice {
    pub value: f64,
}
