use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program::invoke;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::system_program;
use solana_program::sysvar::Sysvar;

use crate::create_pda::create_pda_account;

pub fn execute_create_referral(accounts: &[AccountInfo], instruction_data: &[u8]) -> ProgramResult {
    if let [payer, referrer, vault, mint, system_program, token_program] = accounts {
        // verify token program is passed
        if !spl_token::ID.eq(token_program.key) {
            return Err(ProgramError::IncorrectProgramId);
        }

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

        create_pda_account(
            payer,
            &Rent::get()?,
            spl_token::state::Account::LEN,
            &spl_token::ID,
            system_program,
            vault,
            &vault_seeds,
        )?;

        let initialize_ix = spl_token::instruction::initialize_account3(
            &spl_token::ID,
            vault.key,
            mint.key,
            vault.key,
        )?;

        let initialize_account_infos = [vault.clone(), mint.clone(), token_program.clone()];
        invoke(&initialize_ix, &initialize_account_infos)?;

        Ok(())
    } else {
        Err(ProgramError::NotEnoughAccountKeys)
    }
}
