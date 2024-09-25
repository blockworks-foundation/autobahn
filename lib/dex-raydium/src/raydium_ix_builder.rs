use crate::internal::state::AmmInfo;
use crate::raydium_edge::RaydiumEdgeIdentifier;
use anchor_lang::Id;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::Token;
use router_lib::dex::{AccountProviderView, DexEdgeIdentifier, SwapInstruction};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;

pub fn build_swap_ix(
    id: &RaydiumEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    amount_in: u64,
    out_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let pool_account = chain_data.account(&id.amm)?;
    let pool = AmmInfo::load_checked(pool_account.account.data())?;

    let minimum_amount_out =
        ((out_amount as f64 * (10_000f64 - max_slippage_bps as f64)) / 10_000f64).floor() as u64;

    let (input_token_mint, output_token_mint) = (id.input_mint(), id.output_mint());

    let (input_token_account, output_token_account) = (
        get_associated_token_address(wallet_pk, &input_token_mint),
        get_associated_token_address(wallet_pk, &output_token_mint),
    );

    let exptected_size = 1 + 8 + 8;
    let mut data = Vec::with_capacity(exptected_size);
    data.extend_from_slice(&[9u8]);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&minimum_amount_out.to_le_bytes());
    assert_eq!(data.len(), exptected_size);

    let result = SwapInstruction {
        instruction: Instruction {
            program_id: crate::ID,
            accounts: vec![
                AccountMeta::new_readonly(Token::id(), false),
                AccountMeta::new(id.amm, false), // Amm Info
                AccountMeta::new_readonly(crate::authority::ID, false), // Amm authority
                AccountMeta::new(id.amm, false), // oo
                AccountMeta::new(pool.coin_vault, false), // coin vault
                AccountMeta::new(pool.pc_vault, false), // pc vault
                AccountMeta::new(id.amm, false), // ob program
                AccountMeta::new(id.amm, false), // ob market
                AccountMeta::new(id.amm, false), // ob bids
                AccountMeta::new(id.amm, false), // ob asks
                AccountMeta::new(id.amm, false), // ob events queue
                AccountMeta::new(id.amm, false), // ob coin
                AccountMeta::new(id.amm, false), // ob pc
                AccountMeta::new(id.amm, false), // ob signer
                AccountMeta::new(input_token_account, false), // user source token account
                AccountMeta::new(output_token_account, false), // user destination token account
                AccountMeta::new(*wallet_pk, true),
            ],
            data,
        },
        out_pubkey: output_token_account,
        out_mint: output_token_mint,
        in_amount_offset: 1,
        cu_estimate: Some(40_000),
    };

    Ok(result)
}

pub fn _authority_id(program_id: &Pubkey, my_info: &Pubkey, nonce: u8) -> anyhow::Result<Pubkey> {
    let Ok(pda) =
        Pubkey::create_program_address(&[&my_info.to_bytes()[..32], &[nonce]], program_id)
    else {
        anyhow::bail!("can't find authority id for raydium swap")
    };

    Ok(pda)
}
