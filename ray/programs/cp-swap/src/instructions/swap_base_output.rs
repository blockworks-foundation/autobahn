use super::swap_base_input::Swap;
use crate::curve::{calculator::CurveCalculator, TradeDirection};
use crate::error::ErrorCode;
use crate::states::*;
use crate::utils::token::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program;

pub fn swap_base_output(
    ctx: Context<Swap>,
    max_amount_in: u64,
    amount_out_less_fee: u64,
) -> Result<()> {
    let block_timestamp = solana_program::clock::Clock::get()?.unix_timestamp as u64;
    let pool_id = ctx.accounts.pool_state.key();
    let pool_state = &mut ctx.accounts.pool_state.load_mut()?;
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)
        || block_timestamp < pool_state.open_time
    {
        return err!(ErrorCode::NotApproved);
    }
    let out_transfer_fee = get_transfer_inverse_fee(
        &ctx.accounts.output_token_mint.to_account_info(),
        amount_out_less_fee,
    )?;
    let actual_amount_out = amount_out_less_fee.checked_add(out_transfer_fee).unwrap();

    // Calculate the trade amounts
    let (trade_direction, total_input_token_amount, total_output_token_amount) =
        if ctx.accounts.input_vault.key() == pool_state.token_0_vault
            && ctx.accounts.output_vault.key() == pool_state.token_1_vault
        {
            let (total_input_token_amount, total_output_token_amount) = pool_state
                .vault_amount_without_fee(
                    ctx.accounts.input_vault.amount,
                    ctx.accounts.output_vault.amount,
                );

            (
                TradeDirection::ZeroForOne,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else if ctx.accounts.input_vault.key() == pool_state.token_1_vault
            && ctx.accounts.output_vault.key() == pool_state.token_0_vault
        {
            let (total_output_token_amount, total_input_token_amount) = pool_state
                .vault_amount_without_fee(
                    ctx.accounts.output_vault.amount,
                    ctx.accounts.input_vault.amount,
                );

            (
                TradeDirection::OneForZero,
                total_input_token_amount,
                total_output_token_amount,
            )
        } else {
            return err!(ErrorCode::InvalidVault);
        };
    let constant_before = u128::from(total_input_token_amount)
        .checked_mul(u128::from(total_output_token_amount))
        .unwrap();
    let (input_token_creator_rate, input_token_lp_rate) = match trade_direction {
        TradeDirection::ZeroForOne => (
            ctx.accounts.amm_config.token_1_creator_rate,
            ctx.accounts.amm_config.token_1_lp_rate,
        ),
        TradeDirection::OneForZero => (
            ctx.accounts.amm_config.token_0_creator_rate,
            ctx.accounts.amm_config.token_0_lp_rate,
        ),
    };
    let total_fee = input_token_creator_rate + input_token_lp_rate;
    let protocol_fee = total_fee / 10000 * 2;
    let result = CurveCalculator::swap_base_output(
        u128::from(actual_amount_out),
        u128::from(total_input_token_amount),
        u128::from(total_output_token_amount),
        total_fee,
        protocol_fee,
        input_token_creator_rate
    )
    .ok_or(ErrorCode::ZeroTradingTokens)?;

    let constant_after = u128::from(result.new_swap_source_amount)
        .checked_mul(u128::from(result.new_swap_destination_amount))
        .unwrap();

    #[cfg(feature = "enable-log")]
    msg!(
        "source_amount_swapped:{}, destination_amount_swapped:{},constant_before:{},constant_after:{}",
        result.source_amount_swapped,
        result.destination_amount_swapped,
        constant_before,
        constant_after
    );
    require_gte!(constant_after, constant_before);

    // Re-calculate the source amount swapped based on what the curve says
    let (input_transfer_amount, input_transfer_fee) = {
        let source_amount_swapped = u64::try_from(result.source_amount_swapped).unwrap();
        require_gt!(source_amount_swapped, 0);
        let transfer_fee = get_transfer_inverse_fee(
            &ctx.accounts.input_token_mint.to_account_info(),
            source_amount_swapped,
        )?;
        let input_transfer_amount = source_amount_swapped.checked_add(transfer_fee).unwrap();
        require_gte!(
            max_amount_in,
            input_transfer_amount,
            ErrorCode::ExceededSlippage
        );
        (input_transfer_amount, transfer_fee)
    };
    require_eq!(
        u64::try_from(result.destination_amount_swapped).unwrap(),
        actual_amount_out
    );
    let (output_transfer_amount, output_transfer_fee) = (actual_amount_out, out_transfer_fee);

    let protocol_fee = u64::try_from(result.protocol_fee).unwrap();
    let fund_fee = u64::try_from(result.creator_fee).unwrap();

    match trade_direction {
        TradeDirection::ZeroForOne => {
            pool_state.protocol_fees_token_0 = pool_state
                .protocol_fees_token_0
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_0 =
                pool_state.fund_fees_token_0.checked_add(fund_fee).unwrap();
        }
        TradeDirection::OneForZero => {
            pool_state.protocol_fees_token_1 = pool_state
                .protocol_fees_token_1
                .checked_add(protocol_fee)
                .unwrap();
            pool_state.fund_fees_token_1 =
                pool_state.fund_fees_token_1.checked_add(fund_fee).unwrap();
        }
    };

    emit!(SwapEvent {
        pool_id,
        input_vault_before: total_input_token_amount,
        output_vault_before: total_output_token_amount,
        input_amount: u64::try_from(result.source_amount_swapped).unwrap(),
        output_amount: u64::try_from(result.destination_amount_swapped).unwrap(),
        input_transfer_fee,
        output_transfer_fee,
        base_input: false
    });

    transfer_from_user_to_pool_vault(
        ctx.accounts.payer.to_account_info(),
        ctx.accounts.input_token_account.to_account_info(),
        ctx.accounts.input_vault.to_account_info(),
        ctx.accounts.input_token_mint.to_account_info(),
        ctx.accounts.input_token_program.to_account_info(),
        input_transfer_amount,
        ctx.accounts.input_token_mint.decimals,
    )?;

    transfer_from_pool_vault_to_user(
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.output_vault.to_account_info(),
        ctx.accounts.output_token_account.to_account_info(),
        ctx.accounts.output_token_mint.to_account_info(),
        ctx.accounts.output_token_program.to_account_info(),
        output_transfer_amount,
        ctx.accounts.output_token_mint.decimals,
        &[&[crate::AUTH_SEED.as_bytes(), &[pool_state.auth_bump]]],
    )?;

    ctx.accounts.input_vault.reload()?;
    ctx.accounts.output_vault.reload()?;
    let (token_0_price_x64, token_1_price_x64) = if ctx.accounts.input_vault.key()
        == pool_state.token_0_vault
        && ctx.accounts.output_vault.key() == pool_state.token_1_vault
    {
        pool_state.token_price_x32(
            ctx.accounts.input_vault.amount,
            ctx.accounts.output_vault.amount,
        )
    } else if ctx.accounts.input_vault.key() == pool_state.token_1_vault
        && ctx.accounts.output_vault.key() == pool_state.token_0_vault
    {
        pool_state.token_price_x32(
            ctx.accounts.output_vault.amount,
            ctx.accounts.input_vault.amount,
        )
    } else {
        return err!(ErrorCode::InvalidVault);
    };

    ctx.accounts.observation_state.load_mut()?.update(
        oracle::block_timestamp(),
        token_0_price_x64,
        token_1_price_x64,
    );
    pool_state.recent_epoch = Clock::get()?.epoch;

    Ok(())
}
