use crate::edge::{swap_base_input, swap_base_output, GammaEdge, GammaEdgeIdentifier};
use crate::gamma_cp_ix_builder;
use anchor_lang::{AccountDeserialize, Discriminator, Id};
use anchor_spl::token::spl_token::state::AccountState;
use anchor_spl::token::{spl_token, Token};
use anchor_spl::token_2022::spl_token_2022;
use anyhow::Context;
use async_trait::async_trait;
use gamma::program::Gamma;
use gamma::states::{block_timestamp, AmmConfig, ObservationState, PoolState, PoolStatusBitIndex};
use itertools::Itertools;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode,
    MixedDexSubscription, Quote, SwapInstruction,
};
use router_lib::utils;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use solana_sdk::clock::Clock;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::sysvar::SysvarId;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::u64;

pub struct GammaCpDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
    pub needed_accounts: HashSet<Pubkey>,
}

fn get_gamma_authority() -> Pubkey {
    Pubkey::find_program_address(&[&gamma::AUTH_SEED.as_bytes()], &gamma::id()).0
}

#[async_trait]
impl DexInterface for GammaCpDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let pools = fetch_gamma_account::<PoolState>(rpc, Gamma::id(), PoolState::LEN).await?;

        let vaults = pools
            .iter()
            .flat_map(|x| [x.1.token_0_vault, x.1.token_1_vault])
            .collect::<HashSet<_>>();
        let vaults = rpc.get_multiple_accounts(&vaults).await?;
        let banned_vaults = vaults
            .iter()
            .filter(|x| {
                x.1.owner == Token::id()
                    && spl_token::state::Account::unpack(x.1.data()).unwrap().state
                        == AccountState::Frozen
            })
            .map(|x| x.0)
            .collect::<HashSet<_>>();

        let pools = pools
            .iter()
            .filter(|(_pool_pk, pool)| {
                pool.token_0_program == Token::id() && pool.token_1_program == Token::id()
                // TODO Remove filter when 2022 are working
            })
            .filter(|(_pool_pk, pool)| {
                !banned_vaults.contains(&pool.token_0_vault)
                    && !banned_vaults.contains(&pool.token_1_vault)
            })
            .collect_vec();

        let edge_pairs = pools
            .iter()
            .map(|(pool_pk, pool)| {
                (
                    Arc::new(GammaEdgeIdentifier {
                        pool: *pool_pk,
                        mint_a: pool.token_0_mint,
                        mint_b: pool.token_1_mint,
                        is_a_to_b: true,
                    }),
                    Arc::new(GammaEdgeIdentifier {
                        pool: *pool_pk,
                        mint_a: pool.token_1_mint,
                        mint_b: pool.token_0_mint,
                        is_a_to_b: false,
                    }),
                )
            })
            .collect_vec();

        let mut needed_accounts = HashSet::new();

        let edges_per_pk = {
            let mut map = HashMap::new();
            for ((pool_pk, pool), (edge_a_to_b, edge_b_to_a)) in pools.iter().zip(edge_pairs.iter())
            {
                let entry = vec![
                    edge_a_to_b.clone() as Arc<dyn DexEdgeIdentifier>,
                    edge_b_to_a.clone(),
                ];

                utils::insert_or_extend(&mut map, pool_pk, &entry);
                utils::insert_or_extend(&mut map, &pool.amm_config, &entry);
                utils::insert_or_extend(&mut map, &pool.token_0_vault, &entry);
                utils::insert_or_extend(&mut map, &pool.token_1_vault, &entry);

                needed_accounts.insert(*pool_pk);
                needed_accounts.insert(pool.amm_config);
                needed_accounts.insert(pool.token_0_vault);
                needed_accounts.insert(pool.token_1_vault);
                needed_accounts.insert(pool.token_0_mint);
                needed_accounts.insert(pool.token_1_mint);
            }
            map
        };

        Ok(Arc::new(GammaCpDex {
            edges: edges_per_pk,
            needed_accounts,
        }))
    }

    fn name(&self) -> String {
        "Gamma".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        let gamma_authority = get_gamma_authority();
        DexSubscriptionMode::Mixed(MixedDexSubscription {
            accounts: Default::default(),
            programs: HashSet::from([Gamma::id()]),
            // Only subscription to token accounts owned by gamma authority is
            // enough as this will always update when any swap happens on gamma
            token_accounts_for_owner: HashSet::from([gamma_authority]),
        })
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [Gamma::id()].into_iter().collect()
    }

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
        self.edges.clone()
    }

    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>> {
        let id = id.as_any().downcast_ref::<GammaEdgeIdentifier>().unwrap();

        let pool_account = chain_data.account(&id.pool)?;
        let pool = PoolState::try_deserialize(&mut pool_account.account.data())?;
        let config_account = chain_data.account(&pool.amm_config)?;
        let config = AmmConfig::try_deserialize(&mut config_account.account.data())?;

        let vault_0_account = chain_data.account(&pool.token_0_vault)?;
        let vault_0 = spl_token_2022::state::Account::unpack(vault_0_account.account.data())?;

        let vault_1_account = chain_data.account(&pool.token_1_vault)?;
        let vault_1 = spl_token_2022::state::Account::unpack(vault_1_account.account.data())?;

        let mint_0_account = chain_data.account(&pool.token_0_mint)?;
        let mint_1_account = chain_data.account(&pool.token_1_mint)?;
        let transfer_0_fee = crate::edge::get_transfer_config(&mint_0_account)?;
        let transfer_1_fee = crate::edge::get_transfer_config(&mint_1_account)?;

        let observation_state_account = chain_data.account(&pool.observation_key)?;
        let observation_state =
            ObservationState::try_deserialize(&mut observation_state_account.account.data())?;

        Ok(Arc::new(GammaEdge {
            pool,
            config,
            vault_0_amount: vault_0.amount,
            vault_1_amount: vault_1.amount,
            mint_0: transfer_0_fee,
            mint_1: transfer_1_fee,
            observation_state,
        }))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id.as_any().downcast_ref::<GammaEdgeIdentifier>().unwrap();
        let edge = edge.as_any().downcast_ref::<GammaEdge>().unwrap();

        if !edge.pool.get_status_by_bit(PoolStatusBitIndex::Swap) {
            return Ok(Quote {
                in_amount: 0,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: edge.pool.token_0_mint,
            });
        }

        let clock = chain_data.account(&Clock::id()).context("read clock")?;
        let now_ts = clock.account.deserialize_data::<Clock>()?.unix_timestamp as u64;
        if edge.pool.open_time > now_ts {
            return Ok(Quote {
                in_amount: 0,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: edge.pool.token_0_mint,
            });
        }

        let quote = if id.is_a_to_b {
            let result = swap_base_input(
                &edge.pool,
                &edge.config,
                &edge.observation_state,
                edge.pool.token_0_vault,
                edge.vault_0_amount,
                &edge.mint_0,
                edge.pool.token_1_vault,
                edge.vault_1_amount,
                &edge.mint_1,
                in_amount,
                now_ts,
            )?;

            Quote {
                in_amount: result.0,
                out_amount: result.1,
                fee_amount: result.2,
                fee_mint: edge.pool.token_0_mint,
            }
        } else {
            let result = swap_base_input(
                &edge.pool,
                &edge.config,
                &edge.observation_state,
                edge.pool.token_1_vault,
                edge.vault_1_amount,
                &edge.mint_1,
                edge.pool.token_0_vault,
                edge.vault_0_amount,
                &edge.mint_0,
                in_amount,
                now_ts,
            )?;

            Quote {
                in_amount: result.0,
                out_amount: result.1,
                fee_amount: result.2,
                fee_mint: edge.pool.token_1_mint,
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
        let id = id.as_any().downcast_ref::<GammaEdgeIdentifier>().unwrap();
        gamma_cp_ix_builder::build_swap_ix(
            id,
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
        let id = id.as_any().downcast_ref::<GammaEdgeIdentifier>().unwrap();
        let edge = edge.as_any().downcast_ref::<GammaEdge>().unwrap();

        if !edge.pool.get_status_by_bit(PoolStatusBitIndex::Swap) {
            return Ok(Quote {
                in_amount: u64::MAX,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: edge.pool.token_0_mint,
            });
        }

        let clock = chain_data.account(&Clock::id()).context("read clock")?;
        let now_ts = clock.account.deserialize_data::<Clock>()?.unix_timestamp as u64;
        if edge.pool.open_time > now_ts {
            return Ok(Quote {
                in_amount: u64::MAX,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: edge.pool.token_0_mint,
            });
        }

        let quote = if id.is_a_to_b {
            let result = swap_base_output(
                &edge.pool,
                &edge.config,
                &edge.observation_state,
                edge.pool.token_0_vault,
                edge.vault_0_amount,
                &edge.mint_0,
                edge.pool.token_1_vault,
                edge.vault_1_amount,
                &edge.mint_1,
                out_amount,
                now_ts,
            )?;

            Quote {
                in_amount: result.0,
                out_amount: result.1,
                fee_amount: result.2,
                fee_mint: edge.pool.token_0_mint,
            }
        } else {
            let result = swap_base_output(
                &edge.pool,
                &edge.config,
                &edge.observation_state,
                edge.pool.token_1_vault,
                edge.vault_1_amount,
                &edge.mint_1,
                edge.pool.token_0_vault,
                edge.vault_0_amount,
                &edge.mint_0,
                out_amount,
                now_ts,
            )?;

            Quote {
                in_amount: result.0,
                out_amount: result.1,
                fee_amount: result.2,
                fee_mint: edge.pool.token_1_mint,
            }
        };
        Ok(quote)
    }
}

async fn fetch_gamma_account<T: Discriminator + AccountDeserialize>(
    rpc: &mut RouterRpcClient,
    program_id: Pubkey,
    len: usize,
) -> anyhow::Result<Vec<(Pubkey, T)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(len as u64),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, T::DISCRIMINATOR.to_vec())),
        ]),
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
        .map(|account| {
            let pool: T = T::try_deserialize(&mut account.data.as_slice()).unwrap();
            (account.pubkey, pool)
        })
        .collect_vec();

    Ok(result)
}
