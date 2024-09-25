use solana_program::pubkey::*;

use super::utils::*;

#[derive(Debug, Clone, Copy)]
pub struct MintCookie {
    pub authority: TestKeypair,
    pub pubkey: Pubkey,
    pub decimals: u8,
}

#[derive(Debug, Clone)]
pub struct UserCookie {
    pub key: TestKeypair,
    pub token_accounts: Vec<Pubkey>,
}
