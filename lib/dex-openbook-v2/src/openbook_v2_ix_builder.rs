use super::edge::load_anchor;
use crate::edge::OpenbookV2EdgeIdentifier;
use anchor_lang::Id;
use anchor_spl::associated_token::get_associated_token_address;
use itertools::Itertools;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_sdk::clock::Clock;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::sysvar::SysvarId;
use std::str::FromStr;

pub const INCLUDED_MAKERS_COUNT: usize = 2;

pub fn build_swap_ix(
    id: &OpenbookV2EdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    in_amount: u64,
    _out_amount: u64,
    _max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    use openbook_v2::state as o2s;
    let market;
    let other_side;
    {
        market = Box::new(load_anchor::<o2s::Market>(chain_data, &id.market)?);
        other_side = Box::new(load_anchor::<o2s::BookSide>(
            chain_data,
            &if id.is_bid { id.asks } else { id.bids },
        )?);
    };

    // This does not call openbook's PlaceTakeOrder directly: that requires input amounts in lots
    // and the autobahn-executor program can only dynamically adjust native amounts. So instead, we call
    // a wrapper for PlaceTakeOrder on the autobahn-executor program, see execute_openbook_v2_swap().
    let mut data = Vec::with_capacity(16);
    data.push(2u8); // OpenbookV2Swap discriminator
    data.extend_from_slice(&in_amount.to_le_bytes());
    data.push(if id.is_bid { 1 } else { 0 });
    data.push(10u8);

    // Accounts passed to the autobahn-executor are nearly identical to obv2's PlaceTakeOrder
    let accounts = openbook_v2::accounts::PlaceTakeOrder {
        asks: id.asks,
        bids: id.bids,
        event_heap: id.event_heap,
        market: id.market,
        market_authority: market.market_authority,
        market_base_vault: market.market_base_vault,
        market_quote_vault: market.market_quote_vault,
        open_orders_admin: market.open_orders_admin.into(),
        oracle_a: market.oracle_a.into(),
        oracle_b: market.oracle_b.into(),
        penalty_payer: *wallet_pk,
        signer: *wallet_pk,
        system_program: anchor_lang::system_program::System::id(),
        token_program: anchor_spl::token::ID,
        user_base_account: get_associated_token_address(wallet_pk, &market.base_mint),
        user_quote_account: get_associated_token_address(wallet_pk, &market.quote_mint),
    };

    let (out_pubkey, out_mint) = if id.is_bid {
        (accounts.user_base_account, market.base_mint)
    } else {
        (accounts.user_quote_account, market.quote_mint)
    };

    let clock = chain_data.account(&Clock::id())?;
    let now_ts = clock.account.deserialize_data::<Clock>()?.unix_timestamp as u64;

    let mut account_metas = anchor_lang::ToAccountMetas::to_account_metas(&accounts, None);
    let makers = other_side
        .iter_all_including_invalid(now_ts, None)
        .map(|it| it.node.owner)
        .take(INCLUDED_MAKERS_COUNT)
        .unique();
    for maker in makers {
        account_metas.push(AccountMeta {
            pubkey: maker,
            is_signer: false,
            is_writable: true,
        })
    }

    // Main difference from calling PlaceTakeOrder directly: need to pass the obv2 program
    account_metas.insert(
        0,
        AccountMeta {
            pubkey: openbook_v2::id(),
            is_signer: false,
            is_writable: false,
        },
    );

    let instruction = Instruction {
        program_id: Pubkey::from_str("EXECM4wjzdCnrtQjHx5hy1r5k31tdvWBPYbqsjSoPfAh").unwrap(),
        accounts: account_metas,
        data,
    };

    Ok(SwapInstruction {
        instruction,
        out_pubkey,
        out_mint,
        in_amount_offset: 1,
        cu_estimate: Some(85_000),
    })
}
