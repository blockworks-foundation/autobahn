mod instructions;
pub mod logs;
pub mod swap_ix;
pub mod utils;

use instructions::{
    execute_charge_fees, execute_create_referral, execute_openbook_v2_swap, execute_swap_v2,
    execute_swap_v3, execute_withdraw_referral_fees,
};
use solana_program::declare_id;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

#[cfg(not(feature = "no-entrypoint"))]
use solana_program::entrypoint;

declare_id!("AutobNFLMzX1rFCDgwWpwr3ztG5c1oDbSrGq7Jj2LgE");

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

#[repr(u8)]
pub enum Instructions {
    ExecuteSwapV3 = 1,
    OpenbookV2Swap = 2,
    ExecuteSwapV2 = 3,
    ChargeFees = 4,
    CreateReferral = 5,
    WithdrawReferral = 6,
}

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let ix_discriminator = instruction_data[0] & 15;
    let router_version = instruction_data[0] >> 4;

    match ix_discriminator {
        x if x == Instructions::ExecuteSwapV2 as u8 => {
            execute_swap_v2(accounts, &instruction_data[1..], router_version)
        }
        x if x == Instructions::ExecuteSwapV3 as u8 => {
            execute_swap_v3(accounts, &instruction_data[1..], router_version)
        }
        x if x == Instructions::OpenbookV2Swap as u8 => {
            execute_openbook_v2_swap(accounts, &instruction_data[1..])
        }
        x if x == Instructions::ChargeFees as u8 => {
            execute_charge_fees(accounts, &instruction_data[1..])
        }
        x if x == Instructions::CreateReferral as u8 => {
            execute_create_referral(accounts, &instruction_data[1..])
        }
        x if x == Instructions::WithdrawReferral as u8 => {
            execute_withdraw_referral_fees(accounts, &instruction_data[1..])
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn get_balance(account: &AccountInfo) -> Result<u64, ProgramError> {
    let token = spl_token::state::Account::unpack(&account.try_borrow_data()?)?;
    Ok(token.amount)
}

fn get_mint(account: &AccountInfo) -> Result<Pubkey, ProgramError> {
    let token = spl_token::state::Account::unpack(&account.try_borrow_data()?)?;
    Ok(token.mint)
}
