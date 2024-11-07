use crate::internal::accounts::{InvariantSwapAccounts, InvariantSwapParams};
use crate::invariant_edge::{InvariantEdge, InvariantEdgeIdentifier, InvariantSimulationParams};
use anchor_spl::associated_token::get_associated_token_address_with_program_id;
use anyhow::Context;
use invariant_types::math::{get_max_sqrt_price, get_min_sqrt_price};
use router_lib::dex::{AccountProviderView, DexEdgeIdentifier, SwapInstruction};
use sha2::{Digest, Sha256};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use tracing::info;

pub fn build_swap_ix(
    id: &InvariantEdgeIdentifier,
    edge: &InvariantEdge,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    in_amount: u64,
    _out_amount: u64,
    _max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let by_amount_in = true;

    let (source_mint, destination_mint) = (id.input_mint(), id.output_mint());

    let source_acc = chain_data.account(&source_mint)?.account;
    let dest_acc = chain_data.account(&destination_mint)?.account;

    let source_owner = source_acc.owner();
    let destination_owner = dest_acc.owner();
    let (source_account, destination_account) = (
        get_associated_token_address_with_program_id(wallet_pk, &source_mint, &source_owner),
        get_associated_token_address_with_program_id(
            wallet_pk,
            &destination_mint,
            &destination_owner,
        ),
    );

    let sqrt_price_limit = if id.x_to_y {
        get_min_sqrt_price(edge.pool.tick_spacing)?
    } else {
        get_max_sqrt_price(edge.pool.tick_spacing)?
    };

    let invariant_swap_result = &edge
        .simulate_invariant_swap(&InvariantSimulationParams {
            x_to_y: id.x_to_y,
            in_amount,
            sqrt_price_limit,
            by_amount_in,
        })
        .map_err(|e| anyhow::format_err!(e))
        .with_context(|| format!("pool {} x_to_y {}", id.pool, id.x_to_y))?;

    let swap_params = InvariantSwapParams {
        source_account,
        destination_account,
        source_mint,
        destination_mint,
        owner: *wallet_pk,
        invariant_swap_result,
        referral_fee: None,
    };

    let (swap_accounts, _x_to_y) =
        InvariantSwapAccounts::from_pubkeys(chain_data, edge, id.pool, &swap_params)?;
    let metas = swap_accounts.to_account_metas();

    let discriminator = &Sha256::digest(b"global:swap")[0..8];

    let expected_size = 8 + 1 + 8 + 1 + 16;
    let mut ix_data: Vec<u8> = Vec::with_capacity(expected_size);
    ix_data.extend_from_slice(discriminator);
    ix_data.push(id.x_to_y as u8);
    ix_data.extend_from_slice(&in_amount.to_le_bytes());
    ix_data.push(by_amount_in as u8); // by amount in
    ix_data.extend_from_slice(&sqrt_price_limit.v.to_le_bytes());

    assert_eq!(expected_size, ix_data.len());

    let result = SwapInstruction {
        instruction: Instruction {
            program_id: crate::ID,
            accounts: metas,
            data: ix_data,
        },
        out_pubkey: destination_account,
        out_mint: destination_mint,
        in_amount_offset: 9,
        cu_estimate: Some(120000), //p95
    };

    Ok(result)
}
