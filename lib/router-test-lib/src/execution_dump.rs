use std::collections::{HashMap, HashSet};

use serde_derive::{Deserialize, Serialize};
use solana_sdk::account::AccountSharedData;
use solana_sdk::pubkey::Pubkey;

#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionItem {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_amount: u64,
    pub output_amount: u64,
    pub instruction: Vec<u8>,
    pub is_exact_out: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionDump {
    pub wallet_keypair: String,
    pub programs: HashSet<Pubkey>,
    pub cache: Vec<ExecutionItem>,
    pub accounts: HashMap<Pubkey, AccountSharedData>,
}
