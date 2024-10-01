use anchor_lang::Id;
use anchor_spl::token::Token;
use anchor_spl::token_2022::spl_token_2022::extension::transfer_fee::TransferFeeConfig;
use anchor_spl::token_2022::spl_token_2022::extension::{
    BaseStateWithExtensions, StateWithExtensions,
};
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

pub struct RaydiumCpEdgeIdentifier {
    pub pool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub is_a_to_b: bool,
}

impl DexEdgeIdentifier for RaydiumCpEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.pool
    }

    fn desc(&self) -> String {
        format!("RaydiumCp_{}", self.pool)
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

pub struct RaydiumCpEdge {
    pub pool: PoolState,
    pub config: AmmConfig,
    pub vault_0_amount: u64,
    pub vault_1_amount: u64,
    pub mint_0: Option<TransferFeeConfig>,
    pub mint_1: Option<TransferFeeConfig>,
}

impl DexEdge for RaydiumCpEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) fn _get_transfer_config(
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
    let res = panic::catch_unwind(|| {
        let pool_state = pool;
        let block_timestamp = pool_state.open_time + 1; // TODO FAS his is suppose to be the clock
        if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)
            || block_timestamp < pool_state.open_time
        {
            anyhow::bail!("not approved");
        }

        let transfer_fee = get_transfer_fee(input_mint, amount_in)?;

        // Take transfer fees into account for actual amount transferred in
        let actual_amount_in = amount_in.saturating_sub(transfer_fee);
        #[allow(clippy::nonminimal_bool)]
        if !(actual_amount_in > 0) {
            anyhow::bail!("in amount must be gt 0");
        }

        // Calculate the trade amounts

        let (_, total_input_token_amount, total_output_token_amount) = if input_vault_key
            == pool_state.token_0_vault
            && output_vault_key == pool_state.token_1_vault
        {
            let (total_input_token_amount, total_output_token_amount) =
                vault_amount_without_fee(pool_state, input_vault_amount, output_vault_amount)?;

            (
                TradeDirection::ZeroForOne,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else if input_vault_key == pool_state.token_1_vault
            && output_vault_key == pool_state.token_0_vault
        {
            let (total_output_token_amount, total_input_token_amount) =
                vault_amount_without_fee(pool_state, output_vault_amount, input_vault_amount)?;

            (
                TradeDirection::OneForZero,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else {
            anyhow::bail!("Invalid vault");
        };

        let Some(result) = CurveCalculator::swap_base_input(
            u128::from(actual_amount_in),
            u128::from(total_input_token_amount),
            u128::from(total_output_token_amount),
            amm_config.trade_fee_rate,
            amm_config.protocol_fee_rate,
            amm_config.fund_fee_rate,
        ) else {
            return Ok((u64::MAX, 0, 0));
        };

        let (output_transfer_amount, output_transfer_fee) = {
            let amount_out = u64::try_from(result.destination_amount_swapped).unwrap();
            let transfer_fee = get_transfer_fee(output_mint, amount_out)?;
            (amount_out, transfer_fee)
        };

        let trade_fee = u64::try_from(result.trade_fee).unwrap();
        let protocol_fee = u64::try_from(result.protocol_fee).unwrap();
        let fund_fee = u64::try_from(result.fund_fee).unwrap();
        let amount_received = output_transfer_amount
            .checked_sub(output_transfer_fee)
            .unwrap();

        Ok((
            amount_in,
            amount_received,
            trade_fee + protocol_fee + fund_fee,
        ))
    });

    if res.is_ok() {
        res.unwrap()
    } else {
        anyhow::bail!("Something went wrong in raydium cp")
    }
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
    output_mint: &Option<TransferFeeConfig>,
    amount_out: u64,
) -> anyhow::Result<(u64, u64, u64)> {
    let res = panic::catch_unwind(|| {
        let pool_state = pool;
        let block_timestamp = pool_state.open_time + 1; // TODO FAS his is suppose to be the clock
        if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)
            || block_timestamp < pool_state.open_time
        {
            anyhow::bail!("not approved");
        }

        if amount_out == 0 {
            anyhow::bail!("out amount is 0");
        }

        let output_amount = {
            let mut amount_out = u64::try_from(amount_out).unwrap();
            let transfer_fee = get_transfer_fee(output_mint, amount_out)?;
            amount_out = amount_out.checked_add(transfer_fee).unwrap();
            amount_out
        };

        // Calculate the trade amounts
        let (_, total_input_token_amount, total_output_token_amount) = if input_vault_key
            == pool_state.token_0_vault
            && output_vault_key == pool_state.token_1_vault
        {
            let (total_input_token_amount, total_output_token_amount) =
                vault_amount_without_fee(pool_state, input_vault_amount, output_vault_amount)?;

            (
                TradeDirection::ZeroForOne,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else if input_vault_key == pool_state.token_1_vault
            && output_vault_key == pool_state.token_0_vault
        {
            let (total_output_token_amount, total_input_token_amount) =
                vault_amount_without_fee(pool_state, output_vault_amount, input_vault_amount)?;

            (
                TradeDirection::OneForZero,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else {
            anyhow::bail!("Invalid vault");
        };

        if total_output_token_amount < output_amount {
            anyhow::bail!("Vault does not contain enough tokens");
        }

        let Some(result) = CurveCalculator::swap_base_output(
            u128::from(output_amount),
            u128::from(total_input_token_amount),
            u128::from(total_output_token_amount),
            amm_config.trade_fee_rate,
            amm_config.protocol_fee_rate,
            amm_config.fund_fee_rate,
        ) else {
            anyhow::bail!("Can't swap");
        };

        let trade_fee = u64::try_from(result.trade_fee).unwrap();
        let protocol_fee = u64::try_from(result.protocol_fee).unwrap();
        let fund_fee = u64::try_from(result.fund_fee).unwrap();

        Ok((
            result.source_amount_swapped as u64 + trade_fee + protocol_fee + fund_fee,
            result.destination_amount_swapped as u64,
            trade_fee + protocol_fee + fund_fee,
        ))
    });

    if res.is_ok() {
        res.unwrap()
    } else {
        anyhow::bail!("Something went wrong in raydium cp")
    }
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
