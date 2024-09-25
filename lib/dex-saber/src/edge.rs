use std::any::Any;

use anchor_spl::token::spl_token::state::Account;
use solana_program::clock::Clock;
use solana_program::pubkey::Pubkey;
use stable_swap_client::state::SwapInfo;

use router_lib::dex::{DexEdge, DexEdgeIdentifier};

pub struct SaberEdgeIdentifier {
    pub pool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub is_a_to_b: bool,
}

impl DexEdgeIdentifier for SaberEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.pool
    }

    fn desc(&self) -> String {
        format!("Saber_{}", self.pool)
    }

    fn input_mint(&self) -> Pubkey {
        self.mint_a
    }

    fn output_mint(&self) -> Pubkey {
        self.mint_b
    }

    fn accounts_needed(&self) -> usize {
        7
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct SaberEdge {
    pub pool: SwapInfo,
    pub vault_a: Account,
    pub vault_b: Account,
    pub clock: Clock,
}

impl DexEdge for SaberEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
