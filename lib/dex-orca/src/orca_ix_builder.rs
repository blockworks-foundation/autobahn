use super::orca::*;
use crate::orca;
use crate::orca_dex::OrcaEdgeIdentifier;
use anchor_lang::Id;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::Token;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use sha2::{Digest, Sha256};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;
use tracing::debug;
use whirlpools_client::math::sqrt_price_from_tick_index;

pub fn build_swap_ix(
    id: &OrcaEdgeIdentifier,
    account_fetcher: &AccountProviderView,
    wallet_pk: &Pubkey,
    in_amount: u64,
    out_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let whirlpool = load_whirpool(account_fetcher, &id.pool)?;
    let tick_spacing = whirlpool.tick_spacing;
    let a_to_b = id.is_a_to_b;

    let amount = in_amount;
    let other_amount_threshold =
        ((out_amount as f64 * (10_000f64 - max_slippage_bps as f64)) / 10_000f64).floor() as u64;
    let amount_specified_is_input = true; // TODO will need to change to support ExactOut

    let tick_array_starts =
        derive_tick_array_start_indexes(whirlpool.tick_current_index, tick_spacing, a_to_b);
    let tick_arrays =
        fetch_tick_arrays(account_fetcher, &tick_array_starts, &id.pool, &id.program)?;

    let tick_array_end_index = derive_last_tick_in_seq(&tick_arrays, tick_spacing, a_to_b);
    let sqrt_price_limit = sqrt_price_from_tick_index(tick_array_end_index);

    debug!(
        a = %whirlpool.token_mint_a,
        b = %whirlpool.token_mint_b,
        a_to_b = a_to_b,
        in_amount = in_amount,
        out_amount = out_amount,
        other_amount_threshold = other_amount_threshold,
        ?tick_array_starts,
        tick_array_end_index = tick_array_end_index,
        tick_spacing = whirlpool.tick_spacing,
        sqrt_price_limit = sqrt_price_limit,
        "Performing swap"
    );

    // Don't pass addresses of uninitialized tick arrays. Instead, pass the first one again.
    let ta0_pk = tick_array_pk(&id.pool, &id.program, tick_array_starts.0);
    let tick_array_pks = (
        ta0_pk,
        tick_array_starts
            .1
            .map(|t| tick_array_pk(&id.pool, &id.program, t))
            .unwrap_or(ta0_pk),
        tick_array_starts
            .2
            .map(|t| tick_array_pk(&id.pool, &id.program, t))
            .unwrap_or(ta0_pk),
    );

    let tick_array_pks = if tick_arrays.1.is_none() {
        (tick_array_pks.0, tick_array_pks.0, tick_array_pks.2)
    } else {
        (tick_array_pks.0, tick_array_pks.1, tick_array_pks.2)
    };

    let tick_array_pks = if tick_arrays.2.is_none() {
        (tick_array_pks.0, tick_array_pks.1, tick_array_pks.0)
    } else {
        (tick_array_pks.0, tick_array_pks.1, tick_array_pks.2)
    };

    let x = orca::simulate_swap_with_tick_array(
        account_fetcher,
        &id.pool,
        &whirlpool,
        (amount as f64 * 1.5).round() as u64,
        a_to_b,
        amount_specified_is_input,
        false,
        &id.program,
    );
    let y = orca::simulate_swap_with_tick_array(
        account_fetcher,
        &id.pool,
        &whirlpool,
        (amount as f64 * 1.5).round() as u64,
        a_to_b,
        amount_specified_is_input,
        true,
        &id.program,
    );

    let can_swap_using_only_one_tick_array = if a_to_b {
        x.map(|op| op.amount_b).unwrap_or(0) == y.map(|op| op.amount_b).unwrap_or(0)
    } else {
        x.map(|op| op.amount_a).unwrap_or(0) == y.map(|op| op.amount_a).unwrap_or(0)
    };
    let tick_array_pks = if can_swap_using_only_one_tick_array {
        (tick_array_pks.0, tick_array_pks.0, tick_array_pks.0)
    } else {
        tick_array_pks
    };

    let discriminator = Sha256::digest(b"global:swap");

    let exptected_size = 8 + 8 + 8 + 16 + 1 + 1;
    let mut data = Vec::with_capacity(exptected_size);
    data.extend_from_slice(&discriminator.as_slice()[0..8]);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(amount_specified_is_input as u8);
    data.push(a_to_b as u8);

    assert_eq!(data.len(), exptected_size);

    let swap_ix = Instruction {
        program_id: id.program,

        // orca has a different version of anchor, so can't derive account meta
        accounts: vec![
            AccountMeta::new_readonly(Token::id(), false),
            AccountMeta::new(*wallet_pk, true),
            AccountMeta::new(id.pool, false),
            AccountMeta::new(
                get_associated_token_address(wallet_pk, &whirlpool.token_mint_a),
                false,
            ),
            AccountMeta::new(whirlpool.token_vault_a, false),
            AccountMeta::new(
                get_associated_token_address(wallet_pk, &whirlpool.token_mint_b),
                false,
            ),
            AccountMeta::new(whirlpool.token_vault_b, false),
            AccountMeta::new(tick_array_pks.0, false),
            AccountMeta::new(tick_array_pks.1, false),
            AccountMeta::new(tick_array_pks.2, false),
            AccountMeta::new_readonly(
                Pubkey::find_program_address(&[b"oracle", id.pool.as_ref()], &id.program).0,
                false,
            ),
        ],
        data,
    };

    Ok(SwapInstruction {
        instruction: swap_ix,
        out_pubkey: if id.is_a_to_b {
            get_associated_token_address(wallet_pk, &whirlpool.token_mint_b)
        } else {
            get_associated_token_address(wallet_pk, &whirlpool.token_mint_a)
        },
        out_mint: if id.is_a_to_b {
            whirlpool.token_mint_b
        } else {
            whirlpool.token_mint_a
        },
        in_amount_offset: 8,
        cu_estimate: Some(65_000),
    })
}
