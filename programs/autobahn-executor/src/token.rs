use solana_program::account_info::AccountInfo;
use solana_program::program::{invoke, invoke_signed};
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::sysvar::Sysvar;
use spl_token_2022::extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions};

use crate::create_pda::create_pda_account;

pub fn get_balance(account: &AccountInfo) -> Result<u64, ProgramError> {
    match *account.owner {
        spl_token::ID => {
            let token = spl_token::state::Account::unpack(&account.try_borrow_data()?)?;
            Ok(token.amount)
        }
        spl_token_2022::ID => {
            let data = account.data.borrow();
            let token = StateWithExtensions::<spl_token_2022::state::Account>::unpack(&data)?;
            Ok(token.base.amount)
        }
        _ => Err(ProgramError::IllegalOwner),
    }
}

pub fn get_mint(account: &AccountInfo) -> Result<Pubkey, ProgramError> {
    match *account.owner {
        spl_token::ID => {
            let token = spl_token::state::Account::unpack(&account.try_borrow_data()?)?;
            Ok(token.mint)
        }
        spl_token_2022::ID => {
            let data = account.data.borrow();
            let token = StateWithExtensions::<spl_token_2022::state::Account>::unpack(&data)?;
            Ok(token.base.mint)
        }
        _ => Err(ProgramError::IllegalOwner),
    }
}

pub fn get_owner(account: &AccountInfo) -> Result<Pubkey, ProgramError> {
    match *account.owner {
        spl_token::ID => {
            let token = spl_token::state::Account::unpack(&account.try_borrow_data()?)?;
            Ok(token.owner)
        }
        spl_token_2022::ID => {
            let data = account.data.borrow();
            let token = StateWithExtensions::<spl_token_2022::state::Account>::unpack(&data)?;
            Ok(token.base.owner)
        }
        _ => Err(ProgramError::IllegalOwner),
    }
}

pub fn intialize<'a>(
    payer: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    seeds: &[&[u8]],
) -> Result<(), ProgramError> {
    let space = match *token_program.key {
        spl_token::ID => Ok(spl_token::state::Account::LEN),
        spl_token_2022::ID => {
            let mint_data = mint.data.borrow();
            let mint_with_extension =
                StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&mint_data).unwrap();
            let mint_extensions = mint_with_extension.get_extension_types()?;
            let required_extensions =
                ExtensionType::get_required_init_account_extensions(&mint_extensions);
            let space = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Account>(
                &required_extensions,
            )?;
            Ok(space)
        }
        _ => Err(ProgramError::IllegalOwner),
    }?;

    create_pda_account(
        payer,
        &Rent::get()?,
        space,
        token_program.key,
        system_program,
        account,
        seeds,
    )?;

    let initialize_ix = spl_token::instruction::initialize_account3(
        token_program.key,
        account.key,
        mint.key,
        account.key,
    )?;

    let initialize_account_infos = [account.clone(), mint.clone(), token_program.clone()];
    invoke(&initialize_ix, &initialize_account_infos)
}

pub fn transfer<'a>(
    program: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
    amount: u64,
) -> Result<(), ProgramError> {
    match *program.key {
        spl_token::ID => {
            let transfer_ix = spl_token::instruction::transfer(
                program.key,
                source.key,
                destination.key,
                authority.key,
                &[],
                amount,
            )?;
            let transfer_account_infos = [source.clone(), destination.clone(), program.clone(), authority.clone()];
            if signer_seeds.is_empty() {
                invoke(&transfer_ix, &transfer_account_infos)
            } else {
                invoke_signed(&transfer_ix, &transfer_account_infos, &[signer_seeds])
            }
        }
        spl_token_2022::ID => {
            let mint_data = mint.try_borrow_data()?;
            let mint_parsed =
                StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&mint_data)?;
            let transfer_ix = spl_token_2022::instruction::transfer_checked(
                program.key,
                source.key,
                mint.key,
                destination.key,
                authority.key,
                &[],
                amount,
                mint_parsed.base.decimals,
            )?;
            let transfer_account_infos = [source.clone(), destination.clone(), mint.clone(), program.clone(), authority.clone()];
            if signer_seeds.is_empty() {
                invoke(&transfer_ix, &transfer_account_infos)
            } else {
                invoke_signed(&transfer_ix, &transfer_account_infos, &[signer_seeds])
            }
        }
        _ => Err(ProgramError::IncorrectProgramId),
    }
}

pub fn verify_program_id(address: &Pubkey) -> Result<(), ProgramError> {
    if spl_token::ID.eq(address) || spl_token_2022::ID.eq(address) {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}
