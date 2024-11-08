use anchor_lang::__private::bytemuck;
use anchor_lang::{AnchorDeserialize, Discriminator};
use anyhow::Context;
use itertools::Itertools;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::AccountProviderView;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use solana_sdk::commitment_config::CommitmentConfig;
use std::cell::RefCell;
use std::mem;
use tracing::log::debug;
use tracing::{error, trace};
use whirlpools_client::state::Tick;
use whirlpools_client::{
    manager::swap_manager::{swap, PostSwapUpdate},
    math::sqrt_price_from_tick_index,
    state::{TickArray, Whirlpool, MAX_TICK_INDEX, MIN_TICK_INDEX, TICK_ARRAY_SIZE},
    util::SwapTickSequence,
};

type TickArrayStartIndexes = (i32, Option<i32>, Option<i32>);
type TickArrays = (
    RefCell<TickArray>,
    Option<RefCell<TickArray>>,
    Option<RefCell<TickArray>>,
);

/// Given a tick & tick-spacing, derive the start tick of the tick-array that this tick would reside in
pub fn derive_start_tick(curr_tick: i32, tick_spacing: u16) -> i32 {
    let num_of_ticks_in_array = TICK_ARRAY_SIZE * tick_spacing as i32;
    let rem = curr_tick % num_of_ticks_in_array;
    if curr_tick < 0 && rem != 0 {
        curr_tick - rem - num_of_ticks_in_array
    } else {
        curr_tick - rem
    }
}

pub fn derive_first_tick_array_start_tick(curr_tick: i32, tick_spacing: u16, shifted: bool) -> i32 {
    // Shifting when searching to the right, see get_next_init_tick_index() and in_search_range()
    let tick = if shifted {
        curr_tick + tick_spacing as i32
    } else {
        curr_tick
    };
    derive_start_tick(tick, tick_spacing)
}

pub fn derive_tick_array_start_indexes(
    curr_tick: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> TickArrayStartIndexes {
    let ta0_start_index = derive_first_tick_array_start_tick(curr_tick, tick_spacing, !a_to_b);
    let ta1_start_index_opt = derive_next_start_tick_in_seq(ta0_start_index, tick_spacing, a_to_b);
    let ta2_start_index_opt = ta1_start_index_opt
        .and_then(|nsi| derive_next_start_tick_in_seq(nsi, tick_spacing, a_to_b));
    (ta0_start_index, ta1_start_index_opt, ta2_start_index_opt)
}

pub fn derive_last_tick_in_seq(tick_arrays: &TickArrays, tick_spacing: u16, a_to_b: bool) -> i32 {
    let last_tick_array_start = if let Some(ta) = tick_arrays.2.as_ref() {
        ta.borrow().start_tick_index
    } else if let Some(ta) = tick_arrays.1.as_ref() {
        ta.borrow().start_tick_index
    } else {
        tick_arrays.0.borrow().start_tick_index
    };
    derive_last_tick_in_array(last_tick_array_start, tick_spacing, a_to_b)
}

// The last tick included in a tick array
pub fn derive_last_tick_in_array(start_tick: i32, tick_spacing: u16, a_to_b: bool) -> i32 {
    let num_of_ticks_in_array = TICK_ARRAY_SIZE * tick_spacing as i32;
    let potential_last = if a_to_b {
        start_tick
    } else {
        start_tick + num_of_ticks_in_array - 1
    };
    i32::max(i32::min(potential_last, MAX_TICK_INDEX), MIN_TICK_INDEX)
}

pub fn fetch_tick_arrays(
    chain_data: &AccountProviderView,
    tick_array_starts: &TickArrayStartIndexes,
    whirlpool_pk: &Pubkey,
    program_id: &Pubkey,
) -> anyhow::Result<TickArrays> {
    let fetch_tick_array = |tick| {
        let pk = tick_array_pk(whirlpool_pk, program_id, tick);
        let res = chain_data.account(&pk).map(|a| {
            let data = a.account.data();
            if data.len() < mem::size_of::<TickArray>() + 8 {
                if !data.is_empty() {
                    error!("-> size_of::<TickArray> = {}", mem::size_of::<TickArray>());
                    error!("-> data.len() = {}", data.len());
                    error!(
                        "-> for tick array addr={}, whirlpool_pk={}",
                        pk, whirlpool_pk
                    );
                }
                Err(anyhow::format_err!("Invalid account {} ?", pk))
            } else {
                Ok(RefCell::new(*bytemuck::from_bytes::<TickArray>(
                    &data[8..mem::size_of::<TickArray>() + 8],
                )))
            }
        });

        match res {
            Ok(x) => x.ok(),
            Err(_) => None,
        }
    };
    let Some(ta0) = fetch_tick_array(tick_array_starts.0) else {
        anyhow::bail!(
            "can't load first tick_array {} {} for whirlpool_pk {}",
            tick_array_starts.0,
            tick_array_pk(whirlpool_pk, program_id, tick_array_starts.0),
            whirlpool_pk,
        );
    };
    Ok((
        ta0,
        tick_array_starts.1.and_then(fetch_tick_array),
        tick_array_starts.2.and_then(fetch_tick_array),
    ))
}

pub fn derive_next_start_tick_in_seq(
    start_tick: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> Option<i32> {
    let num_of_ticks_in_array = TICK_ARRAY_SIZE * tick_spacing as i32;
    let potential_last = if a_to_b {
        start_tick - num_of_ticks_in_array
    } else {
        start_tick + num_of_ticks_in_array
    };
    if potential_last < MAX_TICK_INDEX && potential_last > MIN_TICK_INDEX {
        Some(potential_last)
    } else {
        None
    }
}

pub fn tick_array_pk(whirlpool: &Pubkey, program_id: &Pubkey, tick: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"tick_array",
            whirlpool.as_ref(),
            tick.to_string().as_bytes(),
        ],
        program_id,
    )
    .0
}

pub fn load_whirpool(
    chain_data: &AccountProviderView,
    whirlpool_pk: &Pubkey,
) -> anyhow::Result<Whirlpool> {
    let whirlpool_account = chain_data.account(whirlpool_pk)?;
    Ok(AnchorDeserialize::deserialize(
        &mut (&whirlpool_account.account.data()[8..]),
    )?)
}

const TICK_ARRAY_SCAN_RANGE: i32 = 500;

pub fn whirlpool_tick_array_pks(
    whirlpool: &Whirlpool,
    whirlpool_pk: &Pubkey,
    program_id: &Pubkey,
) -> Vec<Pubkey> {
    let ticks_per_tick_array = whirlpool.tick_spacing as i32 * TICK_ARRAY_SIZE;
    let current_tick_array_start_index =
        derive_start_tick(whirlpool.tick_current_index, whirlpool.tick_spacing);
    let lowest_tick_array_start_index = current_tick_array_start_index
        .saturating_sub((TICK_ARRAY_SCAN_RANGE / 2).saturating_mul(ticks_per_tick_array));
    let scanned_tick_array_start_indexes = (0..TICK_ARRAY_SCAN_RANGE)
        .map(|i| {
            lowest_tick_array_start_index.saturating_add(i.saturating_mul(ticks_per_tick_array))
        })
        .filter(|i| Tick::check_is_valid_start_tick(*i, whirlpool.tick_spacing))
        .collect_vec();

    debug!(
        "Will use {} tick array count",
        scanned_tick_array_start_indexes.len()
    );

    debug!(
        "cta_sti={current_tick_array_start_index} sta_stis={scanned_tick_array_start_indexes:?}"
    );
    let pks = scanned_tick_array_start_indexes
        .iter()
        .map(|i| tick_array_pk(whirlpool_pk, program_id, *i))
        .collect_vec();

    debug!("sta_pks={pks:?}");

    pks
}

pub fn simulate_swap(
    chain_data: &AccountProviderView,
    whirlpool_pk: &Pubkey,
    whirlpool: &Whirlpool,
    amount: u64,
    a_to_b: bool,
    amount_specificed_is_input: bool,
    program_id: &Pubkey,
) -> anyhow::Result<PostSwapUpdate> {
    simulate_swap_with_tick_array(
        chain_data,
        whirlpool_pk,
        whirlpool,
        amount,
        a_to_b,
        amount_specificed_is_input,
        false,
        program_id,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn simulate_swap_with_tick_array(
    chain_data: &AccountProviderView,
    whirlpool_pk: &Pubkey,
    whirlpool: &Whirlpool,
    amount: u64,
    a_to_b: bool,
    amount_specificed_is_input: bool,
    use_only_first_tick_array: bool,
    program_id: &Pubkey,
) -> anyhow::Result<PostSwapUpdate> {
    let tick_spacing = whirlpool.tick_spacing;

    let tick_array_starts =
        derive_tick_array_start_indexes(whirlpool.tick_current_index, tick_spacing, a_to_b);

    trace!(
        whirlpool.tick_current_index,
        a_to_b,
        ?tick_array_starts,
        "tick array indexes"
    );

    let tick_arrays = fetch_tick_arrays(chain_data, &tick_array_starts, whirlpool_pk, program_id)?;
    let tick_array_end_index = derive_last_tick_in_seq(&tick_arrays, tick_spacing, a_to_b);
    trace!(tick_array_end_index, "end");
    let sqrt_price_limit = sqrt_price_from_tick_index(tick_array_end_index);

    trace!(
        "later ticks: {} {}",
        tick_arrays.1.is_some(),
        tick_arrays.2.is_some()
    );

    let mut swap_tick_sequence = if use_only_first_tick_array {
        SwapTickSequence::new(tick_arrays.0.borrow_mut(), None, None)
    } else {
        SwapTickSequence::new(
            tick_arrays.0.borrow_mut(),
            tick_arrays.1.as_ref().map(|rc| rc.borrow_mut()),
            tick_arrays.2.as_ref().map(|rc| rc.borrow_mut()),
        )
    };

    let swap_update = swap(
        whirlpool,
        &mut swap_tick_sequence,
        amount,
        sqrt_price_limit,
        amount_specificed_is_input,
        a_to_b,
        whirlpool.reward_last_updated_timestamp,
    );

    swap_update.context("whirlpool swap")
}

pub async fn fetch_all_whirlpools(
    rpc: &mut RouterRpcClient,
    program_id: &Pubkey,
    enable_compression: bool,
) -> anyhow::Result<Vec<(Pubkey, Whirlpool)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(whirlpools_client::state::Whirlpool::LEN as u64),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                0,
                whirlpools_client::state::Whirlpool::DISCRIMINATOR.to_vec(),
            )),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::finalized()),
            ..Default::default()
        },
        ..Default::default()
    };
    let whirlpools = rpc
        .get_program_accounts_with_config(program_id, config, enable_compression)
        .await?;
    let result = whirlpools
        .iter()
        .filter_map(|account| {
            let pubkey = account.pubkey;
            let whirlpool: Result<Whirlpool, std::io::Error> =
                AnchorDeserialize::deserialize(&mut &account.data[8..]);
            match whirlpool {
                Ok(whirlpool) => Some((account.pubkey, whirlpool)),
                Err(e) => {
                    error!("Error deserializing whirlpool account : {pubkey:?} error: {e:?}");
                    None
                }
            }
        })
        .collect_vec();
    Ok(result)
}
