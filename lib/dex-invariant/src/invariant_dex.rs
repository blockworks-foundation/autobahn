use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anchor_lang::{prelude::*, AnchorDeserialize};
use anchor_spl::token::spl_token::state::Account;
use anyhow::{bail, Error, Ok};
use async_trait::async_trait;
use invariant_types::{
    decimals::Price,
    math::{calculate_price_sqrt, get_max_tick, get_min_tick},
    structs::{tick, FeeTier, Pool, Tick, Tickmap, TICKMAP_SIZE, TICK_LIMIT},
    ANCHOR_DISCRIMINATOR_SIZE, MAX_SQRT_PRICE, MIN_SQRT_PRICE, TICK_SEED,
};
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode,
    MixedDexSubscription, Quote, SwapInstruction,
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    pubsub_client::ProgramSubscription,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::RpcFilterType,
};
use solana_sdk::{account::ReadableAccount, pubkey::Pubkey};
use tracing::info;

use crate::{
    invariant_edge::{InvariantEdge, InvariantEdgeIdentifier, InvariantSimulationParams},
    invariant_ix_builder::build_swap_ix,
};

pub struct InvariantDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
}

pub enum PriceDirection {
    UP,
    DOWN,
}
const ADDRESS_LIMIT: usize = 20; //amount of accounts that allows for two routes
const CONST_SWAP_ACCOUNTS: usize = 10; //accounts always present during a swap
const SKIPPED_ACCOUNTS: usize = 2; //accounts that are not included in accounts_needed() function (user output ATA and user wallet address)
pub const TICK_CROSSES_PER_ROUTE_IX: usize = ADDRESS_LIMIT + SKIPPED_ACCOUNTS - CONST_SWAP_ACCOUNTS;

impl InvariantDex {
    pub fn deserialize<T>(data: &[u8]) -> anyhow::Result<T>
    where
        T: AnchorDeserialize,
    {
        T::try_from_slice(Self::extract_from_anchor_account(data))
            .map_err(|e| anyhow::anyhow!("Error deserializing account data: {:?}", e))
    }

    fn extract_from_anchor_account(data: &[u8]) -> &[u8] {
        data.split_at(ANCHOR_DISCRIMINATOR_SIZE).1
    }

    pub fn tick_indexes_to_addresses(pool_address: Pubkey, indexes: &[i32]) -> Vec<Pubkey> {
        let pubkeys: Vec<Pubkey> = indexes
            .iter()
            .map(|i| Self::tick_index_to_address(pool_address, *i))
            .collect();
        pubkeys
    }

    pub fn tick_index_to_address(pool_address: Pubkey, i: i32) -> Pubkey {
        let (pubkey, _) = Pubkey::find_program_address(
            &[
                TICK_SEED.as_bytes(),
                pool_address.as_ref(),
                &i.to_le_bytes(),
            ],
            &crate::ID,
        );
        pubkey
    }

    pub fn get_ticks_addresses_around(
        pool: Pool,
        tickmap: Tickmap,
        pool_address: Pubkey,
    ) -> Vec<Pubkey> {
        let above_indexes = Self::find_closest_tick_indexes(
            &pool,
            &tickmap,
            TICK_CROSSES_PER_ROUTE_IX,
            PriceDirection::UP,
        );
        let below_indexes = Self::find_closest_tick_indexes(
            &pool,
            &tickmap,
            TICK_CROSSES_PER_ROUTE_IX,
            PriceDirection::DOWN,
        );
        let all_indexes = [below_indexes, above_indexes].concat();

        Self::tick_indexes_to_addresses(pool_address, &all_indexes)
    }

    pub fn get_closest_ticks_addresses(
        pool: Pool,
        tickmap: Tickmap,
        pool_address: Pubkey,
        direction: PriceDirection,
    ) -> Vec<Pubkey> {
        let indexes =
            Self::find_closest_tick_indexes(&pool, &tickmap, TICK_CROSSES_PER_ROUTE_IX, direction);

        Self::tick_indexes_to_addresses(pool_address, &indexes)
    }

    fn find_closest_tick_indexes(
        pool: &Pool,
        tickmap: &Tickmap,
        amount_limit: usize,
        direction: PriceDirection,
    ) -> Vec<i32> {
        let current: i32 = pool.current_tick_index;
        let tick_spacing: i32 = pool.tick_spacing.into();
        let tickmap = tickmap.bitmap;

        if current % tick_spacing != 0 {
            panic!("Invalid arguments: can't find initialized ticks")
        }
        let mut found: Vec<i32> = Vec::new();
        let current_index = current / tick_spacing + TICK_LIMIT;
        let (mut above, mut below, mut reached_limit) = (current_index + 1, current_index, false);

        while !reached_limit && found.len() < amount_limit {
            match direction {
                PriceDirection::UP => {
                    let value_above: u8 =
                        *tickmap.get((above / 8) as usize).unwrap() & (1 << (above % 8));
                    if value_above != 0 {
                        found.push(above);
                    }
                    reached_limit = above >= TICKMAP_SIZE;
                    above += 1;
                }
                PriceDirection::DOWN => {
                    let value_below: u8 =
                        *tickmap.get((below / 8) as usize).unwrap() & (1 << (below % 8));
                    if value_below != 0 {
                        found.insert(0, below);
                    }
                    reached_limit = below <= 0;
                    below -= 1;
                }
            }
        }

        if let PriceDirection::DOWN = direction {
            // to avoid searching during the swap
            found.reverse();
        }

        found
            .iter()
            .map(|i: &i32| (i - TICK_LIMIT) * tick_spacing)
            .collect()
    }

    fn load_edge(
        id: &InvariantEdgeIdentifier,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<InvariantEdge> {
        let pool_account_data = chain_data.account(&id.pool)?;
        let pool = Self::deserialize::<Pool>(pool_account_data.account.data())?;
        let tickmap_account_data = chain_data.account(&pool.tickmap)?;
        let tickmap = Self::deserialize::<Tickmap>(tickmap_account_data.account.data())?;

        let price_direction = match id.x_to_y {
            true => PriceDirection::DOWN,
            false => PriceDirection::UP,
        };

        let tick_pks = &Self::get_closest_ticks_addresses(pool, tickmap, id.pool, price_direction);
        let mut ticks = Vec::with_capacity(tick_pks.len());

        for tick_pk in tick_pks {
            let tick_data = chain_data.account(&tick_pk)?;
            let tick = Self::deserialize::<Tick>(tick_data.account.data())?;
            ticks.push(tick)
        }

        Ok(InvariantEdge {
            ticks,
            pool,
            tickmap,
        })
    }
}

#[async_trait]
impl DexInterface for InvariantDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let pools = fetch_invariant_accounts(rpc, crate::id()).await?;

        info!("Number of Invariant Pools: {:?}", pools.len());

        let edge_pairs: Vec<(Arc<InvariantEdgeIdentifier>, Arc<InvariantEdgeIdentifier>)> = pools
            .iter()
            .map(|(pool_pk, pool)| {
                (
                    Arc::new(InvariantEdgeIdentifier {
                        pool: *pool_pk,
                        token_x: pool.token_x,
                        token_y: pool.token_y,
                        x_to_y: true,
                    }),
                    Arc::new(InvariantEdgeIdentifier {
                        pool: *pool_pk,
                        token_x: pool.token_x,
                        token_y: pool.token_y,
                        x_to_y: false,
                    }),
                )
            })
            .into_iter()
            .collect();
        let tickmaps = pools.iter().map(|p| p.1.tickmap).collect();
        let tickmaps = rpc.get_multiple_accounts(&tickmaps).await?;

        let edges_per_pk = {
            let mut map = HashMap::new();
            let pools_with_edge_pairs = pools.iter().zip(tickmaps.iter()).zip(edge_pairs.iter());
            for (((pool_pk, pool), (tickmap_pk, tickmap_acc)), (edge_x_to_y, edge_y_to_x)) in
                pools_with_edge_pairs
            {
                let entry: Vec<Arc<dyn DexEdgeIdentifier>> =
                    vec![edge_x_to_y.clone(), edge_y_to_x.clone()];
                map.insert(*pool_pk, entry.clone());
                map.insert(*tickmap_pk, entry.clone());

                let tickmap = Self::deserialize(&tickmap_acc.data)?;
                let indexes = Self::find_closest_tick_indexes(
                    pool,
                    &tickmap,
                    TICKMAP_SIZE as usize,
                    PriceDirection::UP,
                );
                for tick in indexes {
                    map.insert(Self::tick_index_to_address(*pool_pk, tick), entry.clone());
                }
            }
            map
        };

        Ok(Arc::new(InvariantDex {
            edges: edges_per_pk,
        }))
    }

    fn name(&self) -> String {
        "Invariant".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Programs(HashSet::from([crate::ID]))
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [crate::id()].into_iter().collect()
    }

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
        self.edges.clone()
    }

    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>> {
        let id = id
            .as_any()
            .downcast_ref::<InvariantEdgeIdentifier>()
            .unwrap();
        let edge = Self::load_edge(id, chain_data)?;

        Ok(Arc::new(edge))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let edge = edge.as_any().downcast_ref::<InvariantEdge>().unwrap();
        let id = id
            .as_any()
            .downcast_ref::<InvariantEdgeIdentifier>()
            .unwrap();

        let x_to_y = id.x_to_y;
        let sqrt_price_limit = if x_to_y {
            calculate_price_sqrt(get_min_tick(edge.pool.tick_spacing).unwrap())
        } else {
            calculate_price_sqrt(get_max_tick(edge.pool.tick_spacing).unwrap())
        };

        let simulation = edge
            .simulate_invariant_swap(&InvariantSimulationParams {
                x_to_y,
                in_amount,
                sqrt_price_limit,
                by_amount_in: true,
            })
            .map_err(|e| anyhow::format_err!(e))?;

        let fee_mint = if x_to_y { id.token_x } else { id.token_y };

        Ok(Quote {
            in_amount: simulation.in_amount,
            out_amount: simulation.out_amount,
            fee_amount: simulation.fee_amount,
            fee_mint: fee_mint,
        })
    }

    fn build_swap_ix(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
        wallet_pk: &Pubkey,
        in_amount: u64,
        out_amount: u64,
        max_slippage_bps: i32,
    ) -> anyhow::Result<SwapInstruction> {
        let id = {
            id.as_any()
                .downcast_ref::<InvariantEdgeIdentifier>()
                .unwrap()
        };

        let edge = Self::load_edge(id, chain_data)?;

        let swap_ix = build_swap_ix(
            id,
            &edge,
            chain_data,
            wallet_pk,
            in_amount,
            out_amount,
            max_slippage_bps,
        )?;

        Ok(swap_ix)
    }

    fn supports_exact_out(&self, _id: &Arc<dyn DexEdgeIdentifier>) -> bool {
        false
    }

    fn quote_exact_out(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote> {
        let edge = edge.as_any().downcast_ref::<InvariantEdge>().unwrap();
        let id = id
            .as_any()
            .downcast_ref::<InvariantEdgeIdentifier>()
            .unwrap();

        let x_to_y = id.x_to_y;
        let sqrt_price_limit = if x_to_y {
            calculate_price_sqrt(get_min_tick(edge.pool.tick_spacing).unwrap())
        } else {
            calculate_price_sqrt(get_max_tick(edge.pool.tick_spacing).unwrap())
        };

        let simulation = edge
            .simulate_invariant_swap(&InvariantSimulationParams {
                x_to_y,
                in_amount: out_amount,
                sqrt_price_limit,
                by_amount_in: true,
            })
            .map_err(|e| anyhow::format_err!(e))?;

        let fee_mint = if x_to_y { id.token_x } else { id.token_y };

        Ok(Quote {
            in_amount: simulation.in_amount,
            out_amount: simulation.out_amount,
            fee_amount: simulation.fee_amount,
            fee_mint: fee_mint,
        })
    }
}

async fn fetch_invariant_accounts(
    rpc: &mut RouterRpcClient,
    program_id: Pubkey,
) -> anyhow::Result<Vec<(Pubkey, Pool)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(Pool::LEN as u64)]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            ..Default::default()
        },
        ..Default::default()
    };

    let snapshot = rpc
        .get_program_accounts_with_config(&program_id, config)
        .await?;

    let result = snapshot
        .iter()
        .filter_map(|account| {
            let pool = InvariantDex::deserialize::<Pool>(account.data.as_slice());
            pool.ok().map(|x| (account.pubkey, x))
        })
        .collect();

    Ok(result)
}
