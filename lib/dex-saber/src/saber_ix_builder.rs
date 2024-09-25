use crate::edge::SaberEdgeIdentifier;
use anchor_lang::Id;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::Token;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use stable_swap_client::instruction::SwapData;
use stable_swap_client::state::SwapInfo;

pub fn build_swap_ix(
    id: &SaberEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    amount_in: u64,
    out_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let pool_account = chain_data.account(&id.pool)?;
    let pool = SwapInfo::unpack(pool_account.account.data())?;

    let minimum_amount_out =
        ((out_amount as f64 * (10_000f64 - max_slippage_bps as f64)) / 10_000f64).floor() as u64;

    let (input_token_mint, output_token_mint) = if id.is_a_to_b {
        (pool.token_a.mint, pool.token_b.mint)
    } else {
        (pool.token_b.mint, pool.token_a.mint)
    };
    let (input_vault, output_vault) = if id.is_a_to_b {
        (pool.token_a.reserves, pool.token_b.reserves)
    } else {
        (pool.token_b.reserves, pool.token_a.reserves)
    };
    let output_admin_fees_account = if id.is_a_to_b {
        &pool.token_b.admin_fees
    } else {
        &pool.token_a.admin_fees
    };

    let (input_token_account, output_token_account) = (
        get_associated_token_address(wallet_pk, &input_token_mint),
        get_associated_token_address(wallet_pk, &output_token_mint),
    );

    let instruction = stable_swap_client::instruction::SwapInstruction::Swap(SwapData {
        amount_in,
        minimum_amount_out,
    });
    let instruction_data = instruction.pack();

    // 0. `[]`StableSwap
    // 1. `[]` $authority
    // 2. `[signer]` User authority.
    // 3. `[writable]` token_(A|B) SOURCE Account, amount is transferable by $authority,
    // 4. `[writable]` token_(A|B) Base Account to swap INTO.  Must be the SOURCE token.
    // 5. `[writable]` token_(A|B) Base Account to swap FROM.  Must be the DESTINATION token.
    // 6. `[writable]` token_(A|B) DESTINATION Account assigned to USER as the owner.
    // 7. `[writable]` token_(A|B) admin fee Account. Must have same mint as DESTINATION token.
    // 8. `[]` Token program id
    let swap_authority = authority_id(&stable_swap_client::ID, &id.pool, pool.nonce)?;
    let accounts = vec![
        AccountMeta::new_readonly(id.pool, false),
        AccountMeta::new_readonly(swap_authority, false),
        AccountMeta::new_readonly(*wallet_pk, true),
        AccountMeta::new(input_token_account, false),
        AccountMeta::new(input_vault, false),
        AccountMeta::new(output_vault, false),
        AccountMeta::new(output_token_account, false),
        AccountMeta::new(*output_admin_fees_account, false),
        AccountMeta::new_readonly(Token::id(), false),
    ];

    let result = SwapInstruction {
        instruction: Instruction {
            program_id: stable_swap_client::ID,
            accounts,
            data: instruction_data,
        },
        out_pubkey: output_token_account,
        out_mint: output_token_mint,
        in_amount_offset: 1,
        cu_estimate: Some(75_000),
    };

    Ok(result)
}

pub fn authority_id(program_id: &Pubkey, my_info: &Pubkey, nonce: u8) -> anyhow::Result<Pubkey> {
    let Ok(pda) =
        Pubkey::create_program_address(&[&my_info.to_bytes()[..32], &[nonce]], program_id)
    else {
        anyhow::bail!("can't find authority id for saber swap")
    };

    Ok(pda)
}
