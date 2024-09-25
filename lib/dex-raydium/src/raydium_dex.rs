use crate::internal::state::{AmmInfo, AmmStatus};
use crate::internal::SwapDirection;
use crate::raydium_edge::RaydiumEdge;
use crate::raydium_edge::RaydiumEdgeIdentifier;
use crate::{internal, raydium_ix_builder};
use anchor_lang::Id;
use anchor_spl::token::spl_token::state::{Account, AccountState};
use anchor_spl::token::{spl_token, Token};
use async_trait::async_trait;
use chrono::Utc;
use itertools::Itertools;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode,
    MixedDexSubscription, Quote, SwapInstruction,
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use solana_sdk::commitment_config::CommitmentConfig;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::info;

pub struct RaydiumDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
}

#[async_trait]
impl DexInterface for RaydiumDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let pools = fetch_raydium_accounts(rpc, crate::id()).await?;

        info!("Number of raydium AMM: {:?}", pools.len());

        let filtered_pools = pools
            .into_iter()
            .filter(|(_, amm)| {
                AmmStatus::from_u64(amm.status).swap_permission()
                    && !AmmStatus::from_u64(amm.status).orderbook_permission()
            })
            .filter(|(_, amm)| amm.coin_vault_mint != amm.pc_vault_mint)
            .filter(|(_, amm)| {
                amm.status != AmmStatus::WaitingTrade as u64
                    || amm.state_data.pool_open_time < (Utc::now().timestamp() as u64)
            })
            .collect_vec();

        let vaults = filtered_pools
            .iter()
            .flat_map(|x| [x.1.coin_vault, x.1.pc_vault])
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

        let filtered_pools = filtered_pools
            .into_iter()
            .filter(|(_, amm)| {
                !banned_vaults.contains(&amm.coin_vault) && !banned_vaults.contains(&amm.pc_vault)
            })
            .collect_vec();

        info!(
            "Number of raydium AMM post filtering: {:?}",
            filtered_pools.len()
        );

        let edge_pairs = filtered_pools
            .iter()
            .map(|(pool_pk, pool)| {
                (
                    Arc::new(RaydiumEdgeIdentifier {
                        amm: *pool_pk,
                        mint_pc: pool.pc_vault_mint,
                        mint_coin: pool.coin_vault_mint,
                        is_pc_to_coin: true,
                    }),
                    Arc::new(RaydiumEdgeIdentifier {
                        amm: *pool_pk,
                        mint_pc: pool.pc_vault_mint,
                        mint_coin: pool.coin_vault_mint,
                        is_pc_to_coin: false,
                    }),
                )
            })
            .collect_vec();

        let edges_per_pk = {
            let mut map = HashMap::new();
            for ((amm_pk, pool), (edge_a_to_b, edge_b_to_a)) in
                filtered_pools.iter().zip(edge_pairs.iter())
            {
                let entry = vec![
                    edge_a_to_b.clone() as Arc<dyn DexEdgeIdentifier>,
                    edge_b_to_a.clone(),
                ];
                map.insert(*amm_pk, entry.clone());
                map.insert(pool.coin_vault, entry.clone());
                map.insert(pool.pc_vault, entry.clone());
            }
            map
        };

        Ok(Arc::new(RaydiumDex {
            edges: edges_per_pk,
        }))
    }

    fn name(&self) -> String {
        "Raydium".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Mixed(MixedDexSubscription {
            accounts: Default::default(),
            programs: HashSet::from([crate::ID]),
            token_accounts_for_owner: HashSet::from([crate::authority::ID]),
        })
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
        let id = id.as_any().downcast_ref::<RaydiumEdgeIdentifier>().unwrap();

        let amm_account = chain_data.account(&id.amm)?;
        let amm = AmmInfo::load_checked(amm_account.account.data())?;
        let coin_vault_account = chain_data.account(&amm.coin_vault)?;
        let coin_vault = Account::unpack(coin_vault_account.account.data())?;
        let pc_vault_account = chain_data.account(&amm.pc_vault)?;
        let pc_vault = Account::unpack(pc_vault_account.account.data())?;

        Ok(Arc::new(RaydiumEdge {
            amm,
            coin_vault,
            pc_vault,
        }))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id.as_any().downcast_ref::<RaydiumEdgeIdentifier>().unwrap();
        let edge = edge.as_any().downcast_ref::<RaydiumEdge>().unwrap();

        let amm = &edge.amm;
        let coin_vault = &edge.coin_vault;
        let pc_vault = &edge.pc_vault;

        let swap_direction = if id.is_pc_to_coin {
            SwapDirection::PC2Coin
        } else {
            SwapDirection::Coin2PC
        };

        let (out_amount, fee_amount) = internal::processor::simulate_swap_base_in(
            amm,
            coin_vault,
            pc_vault,
            swap_direction,
            in_amount,
        )?;

        let fee_mint = if id.is_pc_to_coin {
            amm.pc_vault_mint
        } else {
            amm.coin_vault_mint
        };

        if !AmmStatus::from_u64(amm.status).swap_permission() {
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
        let id = id.as_any().downcast_ref::<RaydiumEdgeIdentifier>().unwrap();
        raydium_ix_builder::build_swap_ix(
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
        _chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id.as_any().downcast_ref::<RaydiumEdgeIdentifier>().unwrap();
        let edge = edge.as_any().downcast_ref::<RaydiumEdge>().unwrap();

        let amm = &edge.amm;
        let coin_vault = &edge.coin_vault;
        let pc_vault = &edge.pc_vault;

        let swap_direction = if id.is_pc_to_coin {
            SwapDirection::PC2Coin
        } else {
            SwapDirection::Coin2PC
        };

        let (in_amount, fee_amount) = internal::processor::simulate_swap_base_out(
            amm,
            coin_vault,
            pc_vault,
            swap_direction,
            out_amount,
        )?;

        let fee_mint = if id.is_pc_to_coin {
            amm.pc_vault_mint
        } else {
            amm.coin_vault_mint
        };

        if !AmmStatus::from_u64(amm.status).swap_permission() {
            Ok(Quote {
                in_amount: in_amount + fee_amount,
                out_amount,
                fee_amount,
                fee_mint,
            })
        } else {
            Ok(Quote {
                in_amount: in_amount + fee_amount,
                out_amount,
                fee_amount,
                fee_mint,
            })
        }
    }
}

async fn fetch_raydium_accounts(
    rpc: &mut RouterRpcClient,
    program_id: Pubkey,
) -> anyhow::Result<Vec<(Pubkey, AmmInfo)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(
            std::mem::size_of::<AmmInfo>() as u64,
        )]),
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
            let pool = AmmInfo::load_checked(account.data.as_slice());
            pool.ok().map(|x| (account.pubkey, x))
        })
        .collect_vec();

    Ok(result)
}
