use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::system_program;

use crate::{
    logs::{emit_stack, ReferrerWithdrawLog},
    token,
};

pub fn execute_withdraw_referral_fees(
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if let [referrer, vault, mint, referrer_ata, system_program, token_program] = accounts {
        token::verify_program_id(token_program.key)?;

        // verify system program is passed
        if !system_program::ID.eq(system_program.key) {
            return Err(ProgramError::IncorrectProgramId);
        }

        // Check that the referrer is a signer
        if !referrer.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        // Verify the ownership of the referrer_ata
        if token::get_owner(referrer_ata)? != *referrer.key {
            return Err(ProgramError::IllegalOwner);
        }

        let vault_seeds = [
            b"referrer",
            referrer.key.as_ref(),
            mint.key.as_ref(),
            instruction_data,
        ];

        let vault_pda = Pubkey::create_program_address(&vault_seeds, &crate::id())?;

        if !vault_pda.eq(vault.key) {
            return Err(ProgramError::InvalidSeeds);
        }

        // Always withdraw full amount
        let full_amount = token::get_balance(vault)?;
        token::transfer(
            token_program,
            mint,
            vault,
            referrer_ata,
            vault,
            &vault_seeds,
            full_amount,
        )?;

        emit_stack(ReferrerWithdrawLog {
            referer: *referrer.key,
            referer_token_account: *referrer_ata.key,
            amount: full_amount,
        })?;

        Ok(())
    } else {
        Err(ProgramError::NotEnoughAccountKeys)
    }
}
