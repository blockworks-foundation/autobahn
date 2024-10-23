use anchor_lang::Id;
use anchor_spl::token::Token;
use anchor_spl::token_2022::spl_token_2022::extension::transfer_fee::TransferFeeConfig;
use anchor_spl::token_2022::spl_token_2022::extension::{
    BaseStateWithExtensions, StateWithExtensions,
};
use anyhow::anyhow;
use anchor_spl::token_2022::spl_token_2022::state::Mint;
use mango_feeds_connector::chain_data::AccountData;
use raydium_cp_swap::curve::{CurveCalculator, TradeDirection};
use raydium_cp_swap::states::{AmmConfig, PoolState, PoolStatusBitIndex};
use solana_program::clock::Clock;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::Sysvar;
use solana_sdk::account::ReadableAccount;
use std::any::Any;
use std::panic;

use router_lib::dex::{DexEdge, DexEdgeIdentifier};

pub struct GobblerEdgeIdentifier {
    pub pool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub is_a_to_b: bool,
}

impl DexEdgeIdentifier for GobblerEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.pool
    }

    fn desc(&self) -> String {
        format!("Gobbler_{}", self.pool)
    }

    fn input_mint(&self) -> Pubkey {
        self.mint_a
    }

    fn output_mint(&self) -> Pubkey {
        self.mint_b
    }

    fn accounts_needed(&self) -> usize {
        11
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct GobblerEdge {
    pub pool: PoolState,
    pub config: AmmConfig,
    pub vault_0_amount: u64,
    pub vault_1_amount: u64,
    pub mint_0: Option<TransferFeeConfig>,
    pub mint_1: Option<TransferFeeConfig>,
}

impl DexEdge for GobblerEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) fn get_transfer_config(
    mint_account: &AccountData,
) -> anyhow::Result<Option<TransferFeeConfig>> {
    if *mint_account.account.owner() == Token::id() {
        return Ok(None);
    }

    let mint = StateWithExtensions::<Mint>::unpack(mint_account.account.data())?;
    if let Ok(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>() {
        Ok(Some(*transfer_fee_config))
    } else {
        Ok(None)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn swap_base_input(
    pool: &PoolState,
    amm_config: &AmmConfig,
    input_vault_key: Pubkey,
    input_vault_amount: u64,
    input_mint: &Option<TransferFeeConfig>,
    output_vault_key: Pubkey,
    output_vault_amount: u64,
    output_mint: &Option<TransferFeeConfig>,
    amount_in: u64,
) -> anyhow::Result<(u64, u64, u64)> {
    let (total_input_token_amount, total_output_token_amount) = if input_vault_key == pool.token_0_vault {
        vault_amount_without_fee(pool, input_vault_amount, output_vault_amount)?
    } else {
        let (out, inp) = vault_amount_without_fee(pool, output_vault_amount, input_vault_amount)?;
        (inp, out)
    };

    let (input_token_creator_rate, input_token_lp_rate) = if input_vault_key == pool.token_0_vault {
        (amm_config.token_0_creator_rate, amm_config.token_0_lp_rate)
    } else {
        (amm_config.token_1_creator_rate, amm_config.token_1_lp_rate)
    };

    let protocol_fee = (amm_config.token_0_creator_rate 
        + amm_config.token_1_creator_rate
        + amm_config.token_0_lp_rate 
        + amm_config.token_1_lp_rate) / 10000;

    let swap_result = CurveCalculator::swap_base_input(
        amount_in.into(),
        total_input_token_amount.into(),
        total_output_token_amount.into(),
        protocol_fee + input_token_creator_rate + input_token_lp_rate,
        input_token_creator_rate,
        input_token_lp_rate,
    ).ok_or_else(|| anyhow!("Swap calculation failed"))?;

    let amount_received = swap_result.destination_amount_swapped;

    Ok((
        amount_in,
        amount_received.try_into().map_err(|e| anyhow!("Failed to convert amount_received: {}", e))?,
        swap_result.total_fees.try_into().map_err(|e| anyhow!("Failed to convert fees: {}", e))?,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn swap_base_output(
    pool: &PoolState,
    amm_config: &AmmConfig,
    input_vault_key: Pubkey,
    input_vault_amount: u64,
    _input_mint: &Option<TransferFeeConfig>,
    output_vault_key: Pubkey,
    output_vault_amount: u64,
    _output_mint: &Option<TransferFeeConfig>,
    amount_out: u64,
) -> anyhow::Result<(u64, u64, u64)> {
    let (total_input_token_amount, total_output_token_amount) = if input_vault_key == pool.token_0_vault {
        vault_amount_without_fee(pool, input_vault_amount, output_vault_amount)?
    } else {
        let (out, inp) = vault_amount_without_fee(pool, output_vault_amount, input_vault_amount)?;
        (inp, out)
    };

    let (input_token_creator_rate, input_token_lp_rate) = if input_vault_key == pool.token_0_vault {
        (amm_config.token_0_creator_rate, amm_config.token_0_lp_rate)
    } else {
        (amm_config.token_1_creator_rate, amm_config.token_1_lp_rate)
    };

    let protocol_fee = (amm_config.token_0_creator_rate 
        + amm_config.token_1_creator_rate
        + amm_config.token_0_lp_rate 
        + amm_config.token_1_lp_rate) / 10000;

    let swap_result = CurveCalculator::swap_base_output(
        amount_out.into(),
        total_input_token_amount.into(),
        total_output_token_amount.into(),
        protocol_fee + input_token_creator_rate + input_token_lp_rate,
        input_token_creator_rate,
        input_token_lp_rate,
    ).ok_or_else(|| anyhow!("Swap calculation failed"))?;

    Ok((
        swap_result.source_amount_swapped.try_into().map_err(|e| anyhow!("Failed to convert amount_in: {}", e))?,
        amount_out,
        swap_result.total_fees.try_into().map_err(|e| anyhow!("Failed to convert fees: {}", e))?,
    ))
}

pub fn get_transfer_fee(
    mint_info: &Option<TransferFeeConfig>,
    pre_fee_amount: u64,
) -> anchor_lang::Result<u64> {
    let fee = if let Some(transfer_fee_config) = mint_info {
        transfer_fee_config
            .calculate_epoch_fee(Clock::get()?.epoch, pre_fee_amount)
            .unwrap()
    } else {
        0
    };
    Ok(fee)
}

pub fn vault_amount_without_fee(
    pool: &PoolState,
    vault_0: u64,
    vault_1: u64,
) -> anyhow::Result<(u64, u64)> {
    Ok((
        vault_0
            .checked_sub(pool.protocol_fees_token_0 + pool.fund_fees_token_0)
            .ok_or(anyhow::format_err!("invalid sub"))?,
        vault_1
            .checked_sub(pool.protocol_fees_token_1 + pool.fund_fees_token_1)
            .ok_or(anyhow::format_err!("invalid sub"))?,
    ))
}
