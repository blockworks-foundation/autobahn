use std::any::Any;

use s_jup_interface::SPoolJup;
use solana_sdk::pubkey::Pubkey;

use router_lib::dex::{DexEdge, DexEdgeIdentifier};

pub struct InfinityEdge {
    pub data: SPoolJup,
}

pub struct InfinityEdgeIdentifier {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub is_output_lp: bool,
    pub accounts_needed: usize,
}

impl DexEdge for InfinityEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl DexEdgeIdentifier for InfinityEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.input_mint
    }

    fn desc(&self) -> String {
        format!("Infinity_{}", self.input_mint)
    }

    fn input_mint(&self) -> Pubkey {
        self.input_mint
    }

    fn output_mint(&self) -> Pubkey {
        self.output_mint
    }

    fn accounts_needed(&self) -> usize {
        self.accounts_needed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
