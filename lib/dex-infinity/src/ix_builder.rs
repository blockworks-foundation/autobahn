use jupiter_amm_interface::SwapParams;
use s_jup_interface::{SPoolInitAccounts, SPoolInitKeys, SPoolJup};
use sanctum_lst_list::SanctumLstList;
use solana_sdk::{account::Account, pubkey::Pubkey};
use spl_associated_token_account::get_associated_token_address;
use std::collections::HashMap;

use router_lib::dex::{AccountProviderView, SwapInstruction};

use crate::edge::InfinityEdgeIdentifier;

pub fn build_swap_ix(
    id: &InfinityEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    in_amount: u64,
    out_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let program_id = s_controller_lib::program::ID;
    let SanctumLstList { sanctum_lst_list } = SanctumLstList::load();

    let SPoolInitKeys {
        lst_state_list,
        pool_state,
    } = SPoolJup::init_keys(program_id);

    let lst_state_list_account = &chain_data.account(&lst_state_list).unwrap().account;
    let pool_state_account = &chain_data.account(&pool_state).unwrap().account;
    let mut amm: s_jup_interface::SPool<Account, Account> = SPoolJup::from_init_accounts(
        program_id,
        SPoolInitAccounts {
            lst_state_list: lst_state_list_account.clone().into(),
            pool_state: pool_state_account.clone().into(),
        },
        &sanctum_lst_list,
    )?;

    let mut update: HashMap<Pubkey, Account> = HashMap::new();

    for pk in amm.get_accounts_to_update_full().iter() {
        if let Ok(acc) = chain_data.account(pk) {
            update.insert(*pk, acc.account.clone().into());
        }
    }
    amm.update_full(&update)?;

    let (in_mint, out_mint) = (id.input_mint, id.output_mint);

    let in_pubkey = get_associated_token_address(wallet_pk, &in_mint);
    let out_pubkey = get_associated_token_address(wallet_pk, &out_mint);
    let min_out_amount =
        ((out_amount as f64 * (10_000f64 - max_slippage_bps as f64)) / 10_000f64).floor() as u64; // TODO

    let instruction = amm.swap_ix(
        &SwapParams {
            in_amount,
            out_amount: min_out_amount,
            source_mint: in_mint,
            destination_mint: out_mint,
            source_token_account: in_pubkey,
            destination_token_account: out_pubkey,
            token_transfer_authority: *wallet_pk,
            open_order_address: None,
            quote_mint_to_referrer: None,
            jupiter_program_id: &Pubkey::default(),
            missing_dynamic_accounts_as_default: false,
        },
        jupiter_amm_interface::SwapMode::ExactIn,
    )?;

    let in_amount_offset = 6; // same for mint & burn

    return Ok(SwapInstruction {
        instruction,
        out_pubkey,
        out_mint,
        in_amount_offset,
        cu_estimate: None,
    });
}
