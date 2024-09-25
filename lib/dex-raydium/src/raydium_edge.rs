use std::any::Any;

use anchor_spl::token::spl_token::state::Account;
use solana_program::pubkey::Pubkey;

use crate::internal::state::AmmInfo;
use router_lib::dex::{DexEdge, DexEdgeIdentifier};

pub struct RaydiumEdgeIdentifier {
    pub amm: Pubkey,
    pub mint_pc: Pubkey,
    pub mint_coin: Pubkey,
    pub is_pc_to_coin: bool,
}

impl DexEdgeIdentifier for RaydiumEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.amm
    }

    fn desc(&self) -> String {
        format!("Raydium_{}", self.amm)
    }

    fn input_mint(&self) -> Pubkey {
        if self.is_pc_to_coin {
            self.mint_pc
        } else {
            self.mint_coin
        }
    }

    fn output_mint(&self) -> Pubkey {
        if self.is_pc_to_coin {
            self.mint_coin
        } else {
            self.mint_pc
        }
    }

    fn accounts_needed(&self) -> usize {
        6
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct RaydiumEdge {
    pub amm: AmmInfo,
    pub pc_vault: Account,
    pub coin_vault: Account,
}

impl DexEdge for RaydiumEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
