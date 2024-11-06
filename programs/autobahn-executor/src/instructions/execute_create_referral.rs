use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::system_program;

use crate::logs::{emit_stack, CreateReferralLog};
use crate::token;

pub fn execute_create_referral(accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
    if let [payer, referrer, vault, mint, system_program, token_program] = accounts {
        // verify token program is passed
        token::verify_program_id(token_program.key)?;

        // verify system program is passed
        if !system_program::ID.eq(system_program.key) {
            return Err(ProgramError::IncorrectProgramId);
        }

        // verify vault is actually owned by referrer
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

        token::intialize(
            payer,
            system_program,
            token_program,
            mint,
            vault,
            &vault_seeds,
        )?;

        emit_stack(CreateReferralLog {
            referee: *payer.key,
            referer: *referrer.key,
            vault: *vault.key,
            mint: *mint.key,
        })?;

        Ok(())
    } else {
        Err(ProgramError::NotEnoughAccountKeys)
    }
}
