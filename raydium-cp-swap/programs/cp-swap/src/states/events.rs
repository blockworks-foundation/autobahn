use anchor_lang::prelude::*;

/// Emitted when deposit and withdraw
#[event]
#[cfg_attr(feature = "client", derive(Debug))]
pub struct LpChangeEvent {
    #[index]
    pub pool_id: Pubkey,
    pub lp_amount_before: u64,
    /// pool vault sub trade fees
    pub token_0_vault_before: u64,
    /// pool vault sub trade fees
    pub token_1_vault_before: u64,
    /// cacluate result without transfer fee
    pub token_0_amount: u64,
    /// cacluate result without transfer fee
    pub token_1_amount: u64,
    pub token_0_transfer_fee: u64,
    pub token_1_transfer_fee: u64,
    // 0: deposit, 1: withdraw
    pub change_type: u8,
}

/// Emitted when swap
#[event]
#[cfg_attr(feature = "client", derive(Debug))]
pub struct SwapEvent {
    #[index]
    pub pool_id: Pubkey,
    /// pool vault sub trade fees
    pub input_vault_before: u64,
    /// pool vault sub trade fees
    pub output_vault_before: u64,
    /// cacluate result without transfer fee
    pub input_amount: u64,
    /// cacluate result without transfer fee
    pub output_amount: u64,
    pub input_transfer_fee: u64,
    pub output_transfer_fee: u64,
    pub base_input: bool,
}
