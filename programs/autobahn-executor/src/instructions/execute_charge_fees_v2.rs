use crate::logs::{emit_stack, PlatformFeeLog, ReferrerFeeLog};
use crate::token;
use crate::utils::{read_u64, read_u8};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use std::cmp::min;

/// transfers autobahn-executor fee to platform_fee_account and optionally referrer_fee_account
///
/// Instruction data layout
/// Data:
/// - total_fee_amount_native: u64
/// - platform_fee_percent: u8
///
/// If there is a referrer
/// - Platform will get `platform_fee_percent/100 * total_fee_amount_native`
/// - Referrer will get  `(1 - platform_fee_percent/100) * total_fee_amount_native`
///
/// If there is no referrer,
/// - Platform will get `total_fee_amount_native`
pub fn execute_charge_fees_v2(accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
    let (fee_amount, instruction_data) = read_u64(instruction_data);
    let (platform_fee_percent, _) = read_u8(instruction_data);
    let platform_fee_percent = min(100, platform_fee_percent);

    if accounts.len() < 5 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let token_program = &accounts[0];
    let mint_account = &accounts[1];
    let token_account = &accounts[2];
    let platform_fee_account = &accounts[3];
    let signer_account = &accounts[4];

    let has_referrer = accounts.len() == 6;
    let platform_fee_amount = if has_referrer {
        (fee_amount * platform_fee_percent as u64) / 100
    } else {
        fee_amount
    };

    token::transfer(
        token_program,
        mint_account,
        token_account,
        platform_fee_account,
        signer_account,
        &[],
        platform_fee_amount,
    )?;
    emit_stack(PlatformFeeLog {
        user: *signer_account.key,
        platform_token_account: *platform_fee_account.key,
        platform_fee: platform_fee_amount,
    })?;

    if has_referrer {
        let referrer_fee_account = &accounts[4];
        let referrer_fee_amount = fee_amount.saturating_sub(platform_fee_amount);
        token::transfer(
            token_program,
            mint_account,
            token_account,
            referrer_fee_account,
            signer_account,
            &[],
            referrer_fee_amount,
        )?;

        emit_stack(ReferrerFeeLog {
            referee: *signer_account.key,
            referer_token_account: *referrer_fee_account.key,
            referrer_fee: referrer_fee_amount,
        })?;
    }

    Ok(())
}
