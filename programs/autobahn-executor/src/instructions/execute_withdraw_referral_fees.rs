use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::program::invoke_signed;
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_program::system_program;
use spl_token::state::Account as TokenAccount;

use crate::logs::{emit_stack, ReferrerWithdrawLog};

pub fn execute_withdraw_referral_fees(
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if let [referrer, vault, mint, referrer_ata, system_program, token_program] = accounts {
        // verify token program is passed
        if !spl_token::ID.eq(token_program.key) {
            return Err(ProgramError::IncorrectProgramId);
        }

        // verify system program is passed
        if !system_program::ID.eq(system_program.key) {
            return Err(ProgramError::IncorrectProgramId);
        }

        // Check that the referrer is a signer
        if !referrer.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        // Verify the ownership of the referrer_ata
        let referrer_ata_data = TokenAccount::unpack(&referrer_ata.try_borrow_data()?)?;
        if referrer_ata_data.owner != *referrer.key {
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

        // Assume accounts are correctly provided and the `vault` account is an SPL Token account
        let vault_info = vault;

        // Deserialize the token account to get the balance
        let vault_token_account: TokenAccount =
            TokenAccount::unpack(&vault_info.try_borrow_data()?)?;

        // Always go with full amount
        let full_amount = vault_token_account.amount;

        // Create transfer instruction from vault to referrer ATA
        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            vault.key,
            referrer_ata.key,
            vault.key,
            &[],
            full_amount,
        )?;

        let transfer_account_infos = [vault.clone(), referrer_ata.clone(), token_program.clone()];

        invoke_signed(&transfer_ix, &transfer_account_infos, &[&vault_seeds])?;

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
