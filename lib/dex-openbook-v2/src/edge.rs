use router_lib::dex::{AccountProviderView, DexEdge, DexEdgeIdentifier};
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use std::any::Any;
use tracing::warn;

pub struct OpenbookV2EdgeIdentifier {
    pub market: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub event_heap: Pubkey,
    pub is_bid: bool,
    pub account_needed: usize,
}

pub struct OpenbookV2Edge {
    pub market: openbook_v2::state::Market,
    pub bids: Option<openbook_v2::state::BookSide>,
    pub asks: Option<openbook_v2::state::BookSide>,
}

impl DexEdge for OpenbookV2Edge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl DexEdgeIdentifier for OpenbookV2EdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.market
    }

    fn desc(&self) -> String {
        format!("OpenbookV2_{}", self.market)
    }

    fn input_mint(&self) -> Pubkey {
        self.mint_a
    }

    fn output_mint(&self) -> Pubkey {
        self.mint_b
    }

    fn accounts_needed(&self) -> usize {
        self.account_needed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn load_anchor<T: bytemuck::Pod>(
    chain_data: &AccountProviderView,
    address: &Pubkey,
) -> anyhow::Result<T> {
    let account = chain_data.account(address)?;
    let data = bytemuck::try_from_bytes::<T>(&account.account.data()[8..]);
    match data {
        Ok(data) => Ok(*data),
        Err(e) => {
            let size = account.account.data().len();
            warn!(
                "Failed to deserialize account {} (of size={}) {:?}",
                address, size, e
            );
            anyhow::bail!(
                "Failed to deserialize account {} (of size={}) {:?}",
                address,
                size,
                e
            )
        }
    }
}
