use crate::price_feeds::price_feed::PriceUpdate;
use dashmap::DashMap;
use solana_sdk::pubkey::Pubkey;
use std::{str::FromStr, sync::Arc};
use tokio::task::JoinHandle;
use tracing::info;

#[derive(Clone)]
pub struct PriceCache {
    latest_prices: Arc<DashMap<Pubkey, f64>>,
}

impl PriceCache {
    pub fn new(
        mut exit: tokio::sync::broadcast::Receiver<()>,
        mut receiver: tokio::sync::broadcast::Receiver<PriceUpdate>,
    ) -> (PriceCache, JoinHandle<()>) {
        let latest_prices = Arc::new(DashMap::new());
        // seed price cache with stable coin prices, as a point of reference
        latest_prices.insert(
            Pubkey::from_str("AKEWE7Bgh87GPp171b4cJPSSZfmZwQ3KaqYqXoKLNAEE").unwrap(),
            1.0,
        );

        let latest_prices_write = latest_prices.clone();

        let job = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = exit.recv() => {
                        info!("Exit signal received, stopping price cache..");
                        break;
                    },
                    Ok(update) = receiver.recv() => {
                        latest_prices_write.insert(update.mint, update.price);
                    },
                }
            }

            info!("price cache exited")
        });

        (PriceCache { latest_prices }, job)
    }

    pub fn price_ui(&self, mint: Pubkey) -> Option<f64> {
        self.latest_prices.get(&mint).map(|r| *r)
    }
}
