use crate::internal::error::AmmError;
use crate::internal::math::{Calculator, CheckedCeilDiv, SwapDirection, U128};
use crate::internal::state::{AmmInfo, AmmStatus};
use anchor_spl::token::spl_token;
use solana_program::msg;

pub(crate) fn simulate_swap_base_in(
    amm: &AmmInfo,
    amm_coin_vault: &spl_token::state::Account,
    amm_pc_vault: &spl_token::state::Account,
    swap_direction: SwapDirection,
    in_amount: u64,
) -> anyhow::Result<(u64, u64)> {
    if !AmmStatus::from_u64(amm.status).swap_permission() {
        msg!("simulate_swap_base_in: status {}", amm.status);
        return Err(AmmError::InvalidStatus.into());
    }

    let (total_pc_without_take_pnl, total_coin_without_take_pnl) =
        Calculator::calc_total_without_take_pnl_no_orderbook(
            amm_pc_vault.amount,
            amm_coin_vault.amount,
            amm,
        )?;

    let swap_fee = U128::from(in_amount)
        .checked_mul(amm.fees.swap_fee_numerator.into())
        .ok_or(anyhow::format_err!("mul error"))?
        .checked_ceil_div(amm.fees.swap_fee_denominator.into())
        .ok_or(anyhow::format_err!("div error"))?
        .0;
    let swap_in_after_deduct_fee = U128::from(in_amount)
        .checked_sub(swap_fee)
        .ok_or(anyhow::format_err!("sub error"))?;
    let swap_amount_out = Calculator::swap_token_amount_base_in(
        swap_in_after_deduct_fee,
        total_pc_without_take_pnl.into(),
        total_coin_without_take_pnl.into(),
        swap_direction,
    )?
    .as_u64();

    let available_out_token = match swap_direction {
        SwapDirection::PC2Coin => total_coin_without_take_pnl,
        SwapDirection::Coin2PC => total_pc_without_take_pnl,
    };

    let swap_amount_out = swap_amount_out.min(available_out_token);
    Ok((swap_amount_out, swap_fee.as_u64()))
}

pub(crate) fn simulate_swap_base_out(
    amm: &AmmInfo,
    amm_coin_vault: &spl_token::state::Account,
    amm_pc_vault: &spl_token::state::Account,
    swap_direction: SwapDirection,
    out_amount: u64,
) -> anyhow::Result<(u64, u64)> {
    if !AmmStatus::from_u64(amm.status).swap_permission() {
        msg!("simulate_swap_base_out: status {}", amm.status);
        return Err(AmmError::InvalidStatus.into());
    }

    let (total_pc_without_take_pnl, total_coin_without_take_pnl) =
        Calculator::calc_total_without_take_pnl_no_orderbook(
            amm_pc_vault.amount,
            amm_coin_vault.amount,
            amm,
        )?;

    let swap_amount_in = Calculator::swap_token_amount_base_out(
        U128::from(out_amount),
        total_pc_without_take_pnl.into(),
        total_coin_without_take_pnl.into(),
        swap_direction,
    )?
    .as_u64();

    let swap_fee = U128::from(swap_amount_in)
        .checked_mul(amm.fees.swap_fee_numerator.into())
        .unwrap()
        .checked_ceil_div(amm.fees.swap_fee_denominator.into())
        .unwrap()
        .0;

    Ok((swap_amount_in, swap_fee.as_u64()))
}
