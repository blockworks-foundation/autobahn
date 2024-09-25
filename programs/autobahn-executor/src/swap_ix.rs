use crate::utils::{write_bytes, write_u64, write_u8, write_ux16};
use crate::Instructions;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;

pub fn generate_swap_ix_data(
    min_out_amount: u64,
    instructions: &[Instruction],
    in_amount_offsets: &[u16],
    in_account: Pubkey,
    out_accounts: &[Pubkey],
    program_id: Pubkey,
    router_version: u8,
) -> Instruction {
    let mut accounts = vec![];
    accounts.push(AccountMeta {
        pubkey: in_account,
        is_signer: false,
        is_writable: true,
    });

    let mut result = vec![0; 1232];

    let pointer = result.as_mut_slice();
    let mut offset = 0;
    offset += write_u8(
        &mut pointer[offset..],
        (Instructions::ExecuteSwapV3 as u8) + (router_version << 4),
    );
    offset += write_u64(&mut pointer[offset..], min_out_amount);
    offset += write_u8(&mut pointer[offset..], instructions.len() as u8);

    for ((ix, in_amount_offset), out_account) in
        instructions.iter().zip(in_amount_offsets).zip(out_accounts)
    {
        offset += write_ux16(&mut pointer[offset..], ix.data.len() as u16);
        offset += write_ux16(&mut pointer[offset..], *in_amount_offset);
        offset += write_bytes(&mut pointer[offset..], &ix.data);
        offset += write_u8(&mut pointer[offset..], 2 + ix.accounts.len() as u8); // Add 1 for program and 1 for owner (step) out ATA
        accounts.push(AccountMeta {
            pubkey: *out_account,
            is_signer: false,
            is_writable: true,
        });

        accounts.push(AccountMeta {
            pubkey: ix.program_id,
            is_signer: false,
            is_writable: false,
        });

        accounts.extend(ix.accounts.clone());
    }

    let data: Vec<u8> = result[0..offset].to_vec();

    Instruction {
        program_id,
        accounts,
        data,
    }
}
