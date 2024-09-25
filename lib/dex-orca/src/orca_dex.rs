use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use anchor_lang::Id;
use anchor_spl::token::spl_token;
use anchor_spl::token::spl_token::state::AccountState;
use anchor_spl::token_2022::Token2022;
use anyhow::Context;
use itertools::Itertools;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use whirlpools_client::state::Whirlpool;

use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode, Quote,
    SwapInstruction,
};

use crate::orca::{fetch_all_whirlpools, load_whirpool, simulate_swap, whirlpool_tick_array_pks};
use crate::orca_ix_builder;

pub struct OrcaEdgeIdentifier {
    pub pool: Pubkey,
    pub program: Pubkey,
    pub program_name: String,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub is_a_to_b: bool,
}

pub struct OrcaEdge {
    pub whirlpool: Whirlpool,
}

pub struct OrcaDex {
    pub program_id: Pubkey,
    pub program_name: String,
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
}

impl DexEdge for OrcaEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait::async_trait]
impl DexInterface for OrcaDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let mut result = OrcaDex {
            program_id: Pubkey::from_str(options.get("program_id").unwrap()).unwrap(),
            program_name: options.get("program_name").unwrap().clone(),
            edges: HashMap::new(),
        };

        result.edges.extend(
            Self::load_edge_identifiers(rpc, &result.program_name, &result.program_id).await?,
        );

        Ok(Arc::new(result))
    }

    fn name(&self) -> String {
        self.program_name.clone()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Programs([self.program_id].into_iter().collect())
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [self.program_id].into_iter().collect()
    }

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
        self.edges.clone()
    }

    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>> {
        let wp = load_whirpool(chain_data, &id.key())?;
        Ok(Arc::new(OrcaEdge { whirlpool: wp }))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let edge = edge.as_any().downcast_ref::<OrcaEdge>().unwrap();
        let id = id.as_any().downcast_ref::<OrcaEdgeIdentifier>().unwrap();

        let whirlpool = &edge.whirlpool;
        let update = simulate_swap(
            chain_data,
            &id.pool,
            whirlpool,
            in_amount,
            id.is_a_to_b,
            true,
            &self.program_id,
        )
        .with_context(|| format!("swap on {}", id.desc()))?;
        let fees = (whirlpool.fee_rate as f64) / 1_000_000.0 * in_amount as f64;
        let fees = fees.round() as u64;

        let quote = if id.is_a_to_b {
            Quote {
                in_amount: update.amount_a,
                out_amount: update.amount_b,
                fee_amount: fees,
                fee_mint: whirlpool.token_mint_a,
            }
        } else {
            Quote {
                in_amount: update.amount_b,
                out_amount: update.amount_a,
                fee_amount: fees,
                fee_mint: whirlpool.token_mint_b,
            }
        };
        Ok(quote)
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
        orca_ix_builder::build_swap_ix(
            id.as_any().downcast_ref::<OrcaEdgeIdentifier>().unwrap(),
            chain_data,
            wallet_pk,
            in_amount,
            out_amount,
            max_slippage_bps,
        )
    }

    fn supports_exact_out(&self, _id: &Arc<dyn DexEdgeIdentifier>) -> bool {
        true
    }

    fn quote_exact_out(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote> {
        let edge = edge.as_any().downcast_ref::<OrcaEdge>().unwrap();
        let id = id.as_any().downcast_ref::<OrcaEdgeIdentifier>().unwrap();

        let whirlpool = &edge.whirlpool;
        // simulate exact out first
        let update = simulate_swap(
            chain_data,
            &id.pool,
            whirlpool,
            out_amount,
            id.is_a_to_b,
            false,
            &self.program_id,
        )
        .with_context(|| format!("swap on {}", id.desc()))?;

        let in_amount = if id.is_a_to_b {
            update.amount_a
        } else {
            update.amount_b
        };

        let fees = (whirlpool.fee_rate as f64) / 1_000_000.0 * in_amount as f64;
        let fees = fees.round() as u64;

        let quote = if id.is_a_to_b {
            Quote {
                in_amount: update.amount_a,
                out_amount: update.amount_b,
                fee_amount: fees,
                fee_mint: whirlpool.token_mint_a,
            }
        } else {
            Quote {
                in_amount: update.amount_b,
                out_amount: update.amount_a,
                fee_amount: fees,
                fee_mint: whirlpool.token_mint_b,
            }
        };
        Ok(quote)
    }
}

impl OrcaDex {
    async fn load_edge_identifiers(
        rpc: &mut RouterRpcClient,
        program_name: &str,
        program_id: &Pubkey,
    ) -> anyhow::Result<HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>> {
        let whirlpools = fetch_all_whirlpools(rpc, program_id).await?;

        let vaults = whirlpools
            .iter()
            .flat_map(|x| [x.1.token_vault_a, x.1.token_vault_b])
            .collect::<HashSet<_>>();

        let vaults = rpc.get_multiple_accounts(&vaults).await?;
        let banned_vaults = vaults
            .iter()
            .filter(|x| {
                x.1.owner == Token2022::id()
                    || spl_token::state::Account::unpack(x.1.data()).unwrap().state
                        == AccountState::Frozen
            })
            .map(|x| x.0)
            .collect::<HashSet<_>>();

        let filtered_pools = whirlpools
            .into_iter()
            .filter(|(_wp_pk, wp)| {
                !banned_vaults.contains(&wp.token_vault_a)
                    && !banned_vaults.contains(&wp.token_vault_b)
            })
            .collect_vec();

        // TODO: actually need to dynamically adjust subscriptions based on the tick?
        let tick_arrays = filtered_pools
            .iter()
            .map(|(pk, wp)| whirlpool_tick_array_pks(wp, pk, program_id))
            .collect_vec();

        let edge_pairs = filtered_pools
            .iter()
            .map(|(wp_pk, wp)| {
                (
                    Arc::new(OrcaEdgeIdentifier {
                        pool: *wp_pk,
                        program: *program_id,
                        program_name: program_name.to_string(),
                        input_mint: wp.token_mint_a,
                        output_mint: wp.token_mint_b,
                        is_a_to_b: true,
                    }),
                    Arc::new(OrcaEdgeIdentifier {
                        pool: *wp_pk,
                        program: *program_id,
                        program_name: program_name.to_string(),
                        input_mint: wp.token_mint_b,
                        output_mint: wp.token_mint_a,
                        is_a_to_b: false,
                    }),
                )
            })
            .collect_vec();

        // We want to know what edge needs an update when an account is updated.
        // So create a map from tick_array pks and the whirlpool pk to the target.
        let edges_per_pk = {
            let mut map = HashMap::new();
            for (((wp_pk, _wp), tick_arrays), (edge_a_to_b, edge_b_to_a)) in filtered_pools
                .iter()
                .zip(tick_arrays.iter())
                .zip(edge_pairs.iter())
            {
                let entry = vec![
                    edge_a_to_b.clone() as Arc<dyn DexEdgeIdentifier>,
                    edge_b_to_a.clone(),
                ];
                map.insert(*wp_pk, entry.clone());
                for tick_array in tick_arrays {
                    map.insert(*tick_array, entry.clone());
                }
            }
            map
        };

        Ok(edges_per_pk)
    }
}

impl DexEdgeIdentifier for OrcaEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.pool
    }

    fn desc(&self) -> String {
        format!("{}_{}", self.program_name, self.pool)
    }

    fn input_mint(&self) -> Pubkey {
        self.input_mint
    }

    fn output_mint(&self) -> Pubkey {
        self.output_mint
    }

    fn accounts_needed(&self) -> usize {
        9
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
