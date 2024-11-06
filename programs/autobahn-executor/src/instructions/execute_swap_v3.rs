use crate::logs::{emit_stack, SwapEvent};
use crate::token;
use crate::utils::{read_bytes, read_u64, read_u8, read_ux16};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::msg;
use solana_program::program::invoke;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

/// Instruction data layout
/// first account is owner input_token_account
/// - min_out_amount: u64
/// - number of ix: u8
/// - instructions:
///     - ix_size: ux16 (see `read_ux16`)
///     - in_amount_offset: ux16
///     - ix_data: [u8; ix_size]
///     - ix_accounts:
///         - ix_account_count: u8
///     then we have amm program account + out account + ix accounts (in order)
pub fn execute_swap_v3(
    accounts: &[AccountInfo],
    instruction_data: &[u8],
    router_version: u8,
) -> ProgramResult {
    let (min_out_amount, instruction_data) = read_u64(instruction_data);
    let (number_of_ix, instruction_data) = read_u8(instruction_data);

    msg!(
        "Router v={} - Swap in {} hop(s) - expected min out amount {}",
        router_version,
        number_of_ix,
        min_out_amount,
    );

    let mut in_amount = 0u64;
    let mut in_mint = token::get_mint(&accounts[0])?;
    let mut ix_account_index = 1usize;
    let mut ext_instruction_data = instruction_data;

    let mut input_amount = 0u64;
    let mut input_mint = Pubkey::default();
    let input_token_account_balance = token::get_balance(&accounts[0])?;
    let mut output_amount = 0u64;
    let mut output_mint = Pubkey::default();

    for ix_index in 0..number_of_ix {
        let instruction_data = ext_instruction_data;
        let (ix_size, instruction_data) = read_ux16(instruction_data);
        let (in_amount_offset, instruction_data) = read_ux16(instruction_data);
        let (ix_data, instruction_data) = read_bytes(ix_size as usize, instruction_data);

        let (ix_account_count, instruction_data) = read_u8(instruction_data);

        let ix_account_count = ix_account_count as usize;
        let ix_accounts: Vec<AccountInfo> =
            accounts[ix_account_index..ix_account_index + ix_account_count].to_vec();

        let mut ix_data = ix_data.to_vec();

        if ix_index > 0 {
            let in_amount_offset = in_amount_offset as usize;
            let in_amount_override = in_amount.to_le_bytes();
            ix_data[in_amount_offset..in_amount_offset + 8].copy_from_slice(&in_amount_override);
        }

        let instruction = Instruction {
            program_id: *ix_accounts[1].key,
            accounts: ix_accounts[2..]
                .iter()
                .map(|ai| AccountMeta {
                    pubkey: *ai.key,
                    is_signer: ai.is_signer,
                    is_writable: ai.is_writable,
                })
                .collect::<Vec<_>>(),
            data: ix_data,
        };

        let ix_token_account = &ix_accounts[0];
        let balance_before = token::get_balance(ix_token_account)?;
        invoke(&instruction, &ix_accounts)?;
        let balance_after = token::get_balance(ix_token_account)?;
        let out_amount = balance_after - balance_before;
        let out_mint = token::get_mint(ix_token_account)?;

        if ix_index == 0 {
            input_amount = input_token_account_balance - token::get_balance(&accounts[0])?;
            input_mint = in_mint;
        }

        in_amount = out_amount;
        in_mint = out_mint;
        ext_instruction_data = instruction_data;
        ix_account_index += ix_accounts.len();
        output_amount = out_amount;
        output_mint = out_mint;
    }

    emit_stack(SwapEvent {
        input_mint,
        input_amount,
        output_mint,
        output_amount,
    })?;

    if in_amount < min_out_amount {
        msg!(
            "Max slippage reached, expected at least {}, got {}",
            min_out_amount,
            in_amount
        );
        Err(ProgramError::Custom(1)) // TODO Error code
    } else {
        Ok(())
    }
}
