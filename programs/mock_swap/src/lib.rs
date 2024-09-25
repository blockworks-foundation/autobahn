use solana_program::account_info::AccountInfo;
use solana_program::entrypoint;
use solana_program::entrypoint::ProgramResult;
use solana_program::msg;
use solana_program::program::invoke_signed;
use solana_program::pubkey::Pubkey;
use spl_token::*;

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction<'a>(
    _program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    let amount_a = u64::from_le_bytes((&instruction_data[0..8]).try_into().unwrap());
    let amount_b = u64::from_le_bytes((&instruction_data[8..16]).try_into().unwrap());

    msg!("mock swap - trying to do {} --> {}", amount_a, amount_b);

    transfer(
        amount_a,
        &accounts[2],
        &accounts[3],
        &accounts[1],
        &accounts[0],
    );

    transfer(
        amount_b,
        &accounts[5],
        &accounts[6],
        &accounts[4],
        &accounts[0],
    );

    Ok(())
}

fn transfer<'a>(
    amount: u64,
    source: &'a AccountInfo<'a>,
    destination: &'a AccountInfo<'a>,
    authority: &'a AccountInfo<'a>,
    token_program: &'a AccountInfo<'a>,
) {
    let ix = spl_token::instruction::transfer(
        token_program.key,
        source.key,
        destination.key,
        authority.key,
        &[],
        amount,
    )
    .unwrap();

    let accounts = [
        source.clone(),
        destination.clone(),
        authority.clone(),
        token_program.clone(),
    ];
    invoke_signed(&ix, &accounts, &[]).unwrap();
}
