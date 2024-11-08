use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::edge::SaberEdge;
use crate::edge::SaberEdgeIdentifier;
use crate::saber_ix_builder;
use anchor_spl::token::spl_token::state::Account;
use anyhow::{bail, Context};
use async_trait::async_trait;
use itertools::Itertools;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode, Quote,
    SwapInstruction,
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_program::clock::Clock;
use solana_program::program_pack::{IsInitialized, Pack};
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::SysvarId;
use solana_sdk::account::ReadableAccount;
use solana_sdk::commitment_config::CommitmentConfig;
use stable_swap_client::state::SwapInfo;
use stable_swap_math::curve::StableSwap;

pub struct SaberDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
}

#[async_trait]
impl DexInterface for SaberDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let pools =
            fetch_saber_account::<SwapInfo>(rpc, stable_swap_client::id(), SwapInfo::LEN).await?;

        let edge_pairs = pools
            .iter()
            .map(|(pool_pk, pool)| {
                (
                    Arc::new(SaberEdgeIdentifier {
                        pool: *pool_pk,
                        mint_a: pool.token_a.mint,
                        mint_b: pool.token_b.mint,
                        is_a_to_b: true,
                    }),
                    Arc::new(SaberEdgeIdentifier {
                        pool: *pool_pk,
                        mint_a: pool.token_b.mint,
                        mint_b: pool.token_a.mint,
                        is_a_to_b: false,
                    }),
                )
            })
            .collect_vec();

        let edges_per_pk = {
            let mut map = HashMap::new();
            for ((pool_pk, pool), (edge_a_to_b, edge_b_to_a)) in pools.iter().zip(edge_pairs.iter())
            {
                let entry = vec![
                    edge_a_to_b.clone() as Arc<dyn DexEdgeIdentifier>,
                    edge_b_to_a.clone(),
                ];
                map.insert(*pool_pk, entry.clone());
                map.insert(pool.token_a.reserves, entry.clone());
                map.insert(pool.token_b.reserves, entry.clone());
            }
            map
        };

        Ok(Arc::new(SaberDex {
            edges: edges_per_pk,
        }))
    }

    fn name(&self) -> String {
        "Saber".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Accounts(self.edges.keys().copied().collect())
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [stable_swap_client::id()].into_iter().collect()
    }

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
        self.edges.clone()
    }

    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>> {
        let id = id.as_any().downcast_ref::<SaberEdgeIdentifier>().unwrap();

        let pool_account = chain_data.account(&id.pool)?;
        let pool = SwapInfo::unpack(pool_account.account.data())?;
        let vault_a_account = chain_data.account(&pool.token_a.reserves)?;
        let vault_a = Account::unpack(vault_a_account.account.data())?;
        let vault_b_account = chain_data.account(&pool.token_b.reserves)?;
        let vault_b = Account::unpack(vault_b_account.account.data())?;
        let clock_account = chain_data.account(&Clock::id()).context("read clock")?;
        let clock = clock_account.account.deserialize_data::<Clock>()?;

        Ok(Arc::new(SaberEdge {
            pool,
            vault_a,
            vault_b,
            clock,
        }))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id.as_any().downcast_ref::<SaberEdgeIdentifier>().unwrap();
        let edge = edge.as_any().downcast_ref::<SaberEdge>().unwrap();

        let pool = &edge.pool;

        let (out_amount, fee_amount) = if id.is_a_to_b {
            simulate_swap(
                pool,
                &edge.vault_a,
                &edge.vault_b,
                edge.clock.unix_timestamp,
                in_amount,
            )?
        } else {
            simulate_swap(
                pool,
                &edge.vault_b,
                &edge.vault_a,
                edge.clock.unix_timestamp,
                in_amount,
            )?
        };

        let fee_mint = if id.is_a_to_b {
            pool.token_b.mint
        } else {
            pool.token_a.mint
        };

        if pool.is_paused {
            Ok(Quote {
                in_amount,
                out_amount: 0,
                fee_amount,
                fee_mint,
            })
        } else {
            Ok(Quote {
                in_amount,
                out_amount,
                fee_amount,
                fee_mint,
            })
        }
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
        let id = id.as_any().downcast_ref::<SaberEdgeIdentifier>().unwrap();
        saber_ix_builder::build_swap_ix(
            id,
            chain_data,
            wallet_pk,
            in_amount,
            out_amount,
            max_slippage_bps,
        )
    }

    fn supports_exact_out(&self, _id: &Arc<dyn DexEdgeIdentifier>) -> bool {
        false
    }

    fn quote_exact_out(
        &self,
        _id: &Arc<dyn DexEdgeIdentifier>,
        _edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        _out_amount: u64,
    ) -> anyhow::Result<Quote> {
        bail!("exact out not supported")
    }
}

async fn fetch_saber_account<T: Pack + IsInitialized>(
    rpc: &mut RouterRpcClient,
    program_id: Pubkey,
    len: usize,
) -> anyhow::Result<Vec<(Pubkey, T)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(len as u64)]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::finalized()),
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
            let pool = T::unpack(account.data.as_slice());
            pool.ok().map(|x| (account.pubkey, x))
        })
        .collect_vec();

    Ok(result)
}

fn simulate_swap(
    token_swap: &SwapInfo,
    swap_source_account: &Account,
    swap_destination_account: &Account,
    unix_timestamp: i64,
    amount_in: u64,
) -> anyhow::Result<(u64, u64)> {
    let invariant = StableSwap::new(
        token_swap.initial_amp_factor,
        token_swap.target_amp_factor,
        unix_timestamp,
        token_swap.start_ramp_ts,
        token_swap.stop_ramp_ts,
    );
    let Some(result) = invariant.swap_to(
        amount_in,
        swap_source_account.amount,
        swap_destination_account.amount,
        &token_swap.fees,
    ) else {
        anyhow::bail!("Invalid saber swap");
    };

    let amount_swapped = result.amount_swapped;
    let fees = result.fee + result.admin_fee;
    Ok((amount_swapped, fees))
}
