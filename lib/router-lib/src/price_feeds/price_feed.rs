use serde_derive::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use router_config_lib::PriceFeedConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    pub mint: Pubkey,
    pub price: f64,
}

pub trait PriceFeed {
    fn start(
        configuration: PriceFeedConfig,
        exit: broadcast::Receiver<()>,
    ) -> (impl PriceFeed, JoinHandle<()>)
    where
        Self: Sized;
    fn receiver(&mut self) -> broadcast::Receiver<PriceUpdate>;
    fn register_mint_sender(&self) -> async_channel::Sender<Pubkey>;
}
