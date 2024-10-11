use anchor_lang::prelude::*;

pub const AMM_CONFIG_SEED: &str = "amm_config";

/// Holds the current owner of the factory
#[account]
#[derive(Default, Debug)]
pub struct AmmConfig {
    /// Bump to identify PDA
    pub bump: u8,
    /// Status to control if new pool can be create
    pub disable_create_pool: bool,
    /// Config index
    pub index: u64,
    /// The trade fee, denominated in hundredths of a bip (10^-6)
    pub token_1_lp_rate: u64,
    /// The protocol fee
    pub token_0_lp_rate: u64,
    /// The fund fee, denominated in hundredths of a bip (10^-6)
    pub token_0_creator_rate: u64,
    /// Fee for create a new pool
    pub token_1_creator_rate: u64,
    /// Address of the protocol fee owner
    pub protocol_owner: Pubkey,
    /// Address of the fund fee owner
    pub fund_owner: Pubkey,
    /// padding
    pub padding: [u64; 16],
}

impl AmmConfig {
    pub const LEN: usize = 8 + 1 + 8 + 2 + 4 * 8 + 32 * 2 + 8 * 16;
}
