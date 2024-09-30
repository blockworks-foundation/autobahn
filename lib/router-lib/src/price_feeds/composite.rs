use crate::price_feeds::birdeye::BirdeyePriceFeed;
use crate::price_feeds::birdeye_single::BirdeyeSinglePriceFeed;
use crate::price_feeds::fillcity::FillCityPriceFeed;
use crate::price_feeds::price_feed::{PriceFeed, PriceUpdate};
use itertools::Itertools;
use router_config_lib::PriceFeedConfig;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

pub struct CompositePriceFeed {
    admin_channel_sender: async_channel::Sender<Pubkey>,
    update_sender: broadcast::Sender<PriceUpdate>,
}

impl PriceFeed for CompositePriceFeed {
    fn start(
        configuration: PriceFeedConfig,
        mut exit: broadcast::Receiver<()>,
    ) -> (impl PriceFeed, JoinHandle<()>) {
        let (admin_channel_sender, admin_channel_receiver) = async_channel::unbounded::<Pubkey>();
        let (update_sender, _) = broadcast::channel::<PriceUpdate>(10_000);
        let refresh_interval = Duration::from_secs(configuration.refresh_interval_secs);
        let token = configuration.birdeye_token;
        let birdeye_single_mode = configuration.birdeye_single_mode.unwrap_or(false);

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

                        let res = CompositePriceFeed::refresh(&token, birdeye_single_mode, &mints, update_sender_clone.clone()).await;
                        if res.is_err() {
                            error!("Price feed error: {}", res.unwrap_err())
                        }
                    }
                }
            }

            info!("price feed exited")
        });

        let feed = CompositePriceFeed {
            admin_channel_sender,
            update_sender,
        };

        (feed, join_handle)
    }

    fn receiver(&mut self) -> broadcast::Receiver<PriceUpdate> {
        self.update_sender.subscribe()
    }

    fn register_mint_sender(&self) -> async_channel::Sender<Pubkey> {
        self.admin_channel_sender.clone()
    }
}

impl CompositePriceFeed {
    async fn refresh(
        birdeye_token: &String,
        birdeye_single_mode: bool,
        mints: &HashSet<Pubkey>,
        sender: broadcast::Sender<PriceUpdate>,
    ) -> anyhow::Result<()> {
        let mints = mints.iter().copied().collect_vec();
        for chunk in mints.chunks(10_000) {
            let chunk = chunk.iter().copied().collect();

            Self::refresh_chunk(birdeye_token, birdeye_single_mode, &chunk, sender.clone()).await?;
        }

        Ok(())
    }

    async fn refresh_chunk(
        birdeye_token: &String,
        birdeye_single_mode: bool,
        mints: &HashSet<Pubkey>,
        sender: Sender<PriceUpdate>,
    ) -> anyhow::Result<()> {
        let (local_sender, mut local_receiver) = broadcast::channel::<PriceUpdate>(mints.len() + 1);

        info!("Querying price with fill city for {} mints", mints.len());

        let result = FillCityPriceFeed::refresh(mints, local_sender.clone()).await;
        let mints =
            Self::handle_source_results(mints, sender.clone(), &mut local_receiver, result).await?;

        if !mints.is_empty() {
            info!("Querying price with birdeye for {} mints", mints.len());

            let result = if birdeye_single_mode {
                BirdeyeSinglePriceFeed::refresh(
                    birdeye_token.to_string(),
                    &mints,
                    local_sender.clone(),
                )
                .await
            } else {
                BirdeyePriceFeed::refresh(birdeye_token.to_string(), &mints, local_sender.clone())
                    .await
            };
            Self::handle_source_results(&mints, sender.clone(), &mut local_receiver, result)
                .await?;
        }
        Ok(())
    }

    async fn handle_source_results(
        mints: &HashSet<Pubkey>,
        sender: broadcast::Sender<PriceUpdate>,
        local_receiver: &mut broadcast::Receiver<PriceUpdate>,
        result: anyhow::Result<()>,
    ) -> anyhow::Result<HashSet<Pubkey>> {
        match result {
            Ok(_) => {}
            Err(e) => {
                warn!("Failure to refresh prices: {:?}", e);
                return Ok(mints.clone());
            }
        }

        let mut handled_mints = HashSet::new();

        while !local_receiver.is_empty() {
            let price = local_receiver.recv().await?;
            handled_mints.insert(price.mint);

            if sender.receiver_count() == 0 {
                continue;
            }
            sender.send(price)?;
        }

        Ok(mints.difference(&handled_mints).copied().collect())
    }
}
