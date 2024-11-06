pub mod create_pda;
mod instructions;
pub mod logs;
pub mod swap_ix;
pub mod token;
pub mod utils;

use instructions::{
    execute_charge_fees, execute_charge_fees_v2, execute_create_referral, execute_openbook_v2_swap, execute_swap_v2, execute_swap_v3, execute_withdraw_referral_fees
};
use solana_program::declare_id;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

#[cfg(not(feature = "no-entrypoint"))]
use {default_env::default_env, solana_program::entrypoint, solana_security_txt::security_txt};

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "Autobahn Executor",
    project_url: "https://autobahn.ag",
    contacts: "email:security@mango.markets",
    policy: "https://github.com/blockworks-foundation/autobahn/blob/master/SECURITY.md",
    source_code: "https://github.com/blockworks-foundation/autobahn",
    source_revision: default_env!("GITHUB_SHA", ""),
    source_release: default_env!("GITHUB_REF_NAME", "")
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

declare_id!("AutobNFLMzX1rFCDgwWpwr3ztG5c1oDbSrGq7Jj2LgE");

#[repr(u8)]
pub enum Instructions {
    ExecuteSwapV3 = 1,
    OpenbookV2Swap = 2,
    ExecuteSwapV2 = 3,
    ChargeFees = 4,
    CreateReferral = 5,
    WithdrawReferral = 6,
    ChargeFeesV2 = 7,
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
        x if x == Instructions::ChargeFeesV2 as u8 => {
            execute_charge_fees_v2(accounts, &instruction_data[1..])
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
