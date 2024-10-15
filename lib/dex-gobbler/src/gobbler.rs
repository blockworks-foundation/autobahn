use crate::edge::{ swap_base_input, swap_base_output, Direction, GobblerEdge, GobblerEdgeIdentifier, Operation, _get_transfer_config};
use crate::gobbler_ix_builder;
use anchor_lang::{AccountDeserialize, Discriminator, Id};
use anchor_spl::token::spl_token::state::AccountState;
use anchor_spl::token::{spl_token, Token};
use anyhow::Context;
use async_trait::async_trait;
use itertools::Itertools;
use raydium_cp_swap::program::RaydiumCpSwap;
use raydium_cp_swap::states::{AmmConfig, PoolState, PoolStatusBitIndex};
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

pub struct GobblerDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
    pub needed_accounts: HashSet<Pubkey>,
}

#[async_trait]
impl DexInterface for GobblerDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        // Fetch all PoolState accounts
        let pools = fetch_gobbler_pools(rpc).await?;

        // Collect vaults to identify any banned ones (e.g., frozen accounts)
        let vaults = pools
            .iter()
            .flat_map(|(_, pool)| vec![pool.token_0_vault, pool.token_1_vault])
            .collect::<HashSet<_>>();

        let vault_accounts = rpc.get_multiple_accounts(&vaults).await?;
        let banned_vaults = vault_accounts
            .iter()
            .filter(|(_, account)| {
                account.owner == Token::id()
                    && spl_token::state::Account::unpack(account.data())
                        .map(|acc| acc.state == AccountState::Frozen)
                        .unwrap_or(false)
            })
            .map(|(pubkey, _)| *pubkey)
            .collect::<HashSet<_>>();

        // Filter out pools with banned vaults or unsupported token programs
        let valid_pools = pools
            .into_iter()
            .filter(|(_, pool)| {
                pool.token_0_program == Token::id()
                    && pool.token_1_program == Token::id()
                    && !banned_vaults.contains(&pool.token_0_vault)
                    && !banned_vaults.contains(&pool.token_1_vault)
            })
            .collect::<Vec<_>>();

        // Create edge identifiers for each pool
        let mut edge_identifiers = Vec::new();

        for (pool_pk, pool) in &valid_pools {
            // Swap edges between Token A and Token B
            let swap_a_to_b = Arc::new(GobblerEdgeIdentifier {
                pool: *pool_pk,
                mint_a: pool.token_0_mint,
                mint_b: pool.token_1_mint,
                lp_mint: pool.lp_mint,
                operation: Operation::Swap,
                direction: Direction::AtoB,
            });

            let swap_b_to_a = Arc::new(GobblerEdgeIdentifier {
                pool: *pool_pk,
                mint_a: pool.token_1_mint,
                mint_b: pool.token_0_mint,
                lp_mint: pool.lp_mint,
                operation: Operation::Swap,
                direction: Direction::BtoA,
            });

            // Deposit edge from (Token A, Token B) to LP Token
            let deposit_edge = Arc::new(GobblerEdgeIdentifier {
                pool: *pool_pk,
                mint_a: pool.token_0_mint,
                mint_b: pool.token_1_mint,
                lp_mint: pool.lp_mint,
                operation: Operation::Deposit,
                direction: Direction::AandBtoLP,
            });

            // Withdrawal edge from LP Token to (Token A, Token B)
            let withdraw_edge = Arc::new(GobblerEdgeIdentifier {
                pool: *pool_pk,
                mint_a: pool.token_0_mint,
                mint_b: pool.token_1_mint,
                lp_mint: pool.lp_mint,
                operation: Operation::Withdraw,
                direction: Direction::LPtoAandB,
            });

            edge_identifiers.push(swap_a_to_b);
            edge_identifiers.push(swap_b_to_a);
            edge_identifiers.push(deposit_edge);
            edge_identifiers.push(withdraw_edge);
        }

        let mut needed_accounts = HashSet::new();
        let mut edges_per_pk = HashMap::new();

        for (pool_pk, pool) in &valid_pools {
            // Collect all necessary accounts
            needed_accounts.insert(*pool_pk);
            needed_accounts.insert(pool.amm_config);
            needed_accounts.insert(pool.token_0_vault);
            needed_accounts.insert(pool.token_1_vault);
            needed_accounts.insert(pool.lp_mint);
            needed_accounts.insert(pool.token_0_mint);
            needed_accounts.insert(pool.token_1_mint);

            // Map edges to their corresponding public keys
            let edges = edge_identifiers
                .iter()
                .filter(|edge| edge.as_any().downcast_ref::<GobblerEdgeIdentifier>().unwrap().pool == *pool_pk)
                .cloned()
                .collect::<Vec<_>>();
            utils::insert_or_extend(&mut edges_per_pk, pool_pk, &edges.into_iter().map(|edge| edge as Arc<dyn DexEdgeIdentifier>).collect::<Vec<_>>());
        }

        Ok(Arc::new(GobblerDex {
            edges: edges_per_pk,
            needed_accounts,
        }))
    }

    fn name(&self) -> String {
        "Gobbler".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Mixed(MixedDexSubscription {
            accounts: self.needed_accounts.clone(),
            programs: HashSet::from([raydium_cp_swap::id()]),
            token_accounts_for_owner: HashSet::new(),
        })
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [raydium_cp_swap::id()].into_iter().collect()
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
            .downcast_ref::<GobblerEdgeIdentifier>()
            .context("Invalid edge identifier type")?;

        let pool_account = chain_data.account(&id.pool)?;
        let pool = PoolState::try_deserialize(&mut pool_account.account.data())?;

        let config_account = chain_data.account(&pool.amm_config)?;
        let config = AmmConfig::try_deserialize(&mut config_account.account.data())?;

        let vault_0_account = chain_data.account(&pool.token_0_vault)?;
        let vault_0 = spl_token::state::Account::unpack(&vault_0_account.account.data())?;

        let vault_1_account = chain_data.account(&pool.token_1_vault)?;
        let vault_1 = spl_token::state::Account::unpack(&vault_1_account.account.data())?;

        let lp_mint_account = chain_data.account(&pool.lp_mint)?;
        let lp_mint = spl_token::state::Mint::unpack(&lp_mint_account.account.data())?;

        let mint_0_account = chain_data.account(&pool.token_0_mint)?;
        let mint_1_account = chain_data.account(&pool.token_1_mint)?;

        let mint_0 = _get_transfer_config(&mint_0_account)?;
        let mint_1 = _get_transfer_config(&mint_1_account)?;

        Ok(Arc::new(GobblerEdge {
            pool,
            config,
            vault_0_amount: vault_0.amount,
            vault_1_amount: vault_1.amount,
            mint_0,
            mint_1,
            lp_supply: lp_mint.supply,
            operation: id.operation,
            direction: id.direction,
        }))
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id
            .as_any()
            .downcast_ref::<GobblerEdgeIdentifier>()
            .context("Invalid edge identifier type")?;
        let edge = edge.as_any().downcast_ref::<GobblerEdge>().context("Invalid edge type")?;

        match id.operation {
            Operation::Swap => {
                // Handle swap operation
                // Similar to previous implementation
                // ...
            }
            Operation::Deposit => {
                // Handle deposit operation
                // Calculate LP tokens received for given in_amounts of Token A and Token B
                // Return a Quote with LP tokens as out_amount
                // ...
            }
            Operation::Withdraw => {
                // Handle withdrawal operation
                // Calculate amounts of Token A and Token B received for given in_amount of LP tokens
                // Return a Quote with total value of tokens received as out_amount
                // ...
            }
        }

        // Placeholder return until implementation is provided
        Ok(Quote {
            in_amount: 0,
            out_amount: 0,
            fee_amount: 0,
            fee_mint: id.mint_a,
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
        let id = id
            .as_any()
            .downcast_ref::<GobblerEdgeIdentifier>()
            .context("Invalid edge identifier type")?;
    
        match id.operation {
            Operation::Swap => {
                gobbler_ix_builder::build_swap_ix(
                    id,
                    chain_data,
                    wallet_pk,
                    in_amount,
                    out_amount,
                    max_slippage_bps,
                )
            }
            Operation::Deposit => {
                gobbler_ix_builder::build_deposit_ix(
                    id,
                    chain_data,
                    wallet_pk,
                    in_amount, // Adjust as needed
                    max_slippage_bps,
                )
            }
            Operation::Withdraw => {
                gobbler_ix_builder::build_withdraw_ix(
                    id,
                    chain_data,
                    wallet_pk,
                    in_amount, // Adjust as needed
                    max_slippage_bps,
                )
            }
        }
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
        let id = id
            .as_any()
            .downcast_ref::<GobblerEdgeIdentifier>()
            .context("Invalid edge identifier type")?;
        let edge = edge.as_any().downcast_ref::<GobblerEdge>().context("Invalid edge type")?;

        match id.operation {
            Operation::Swap => {
                // Handle swap operation
                // Similar to previous implementation
                // ...
            }
            Operation::Deposit => {
                // Handle deposit operation
                // Calculate amounts of Token A and Token B required to receive given out_amount of LP tokens
                // Return a Quote with total value of tokens required as in_amount
                // ...
            }
            Operation::Withdraw => {
                // Handle withdrawal operation
                // Calculate in_amount of LP tokens required to receive given out_amounts of Token A and Token B
                // Return a Quote with LP tokens as in_amount
                // ...
            }
        }

        // Placeholder return until implementation is provided
        Ok(Quote {
            in_amount: 0,
            out_amount: 0,
            fee_amount: 0,
            fee_mint: id.mint_a,
        })
    }
}

async fn fetch_gobbler_pools(
    rpc: &mut RouterRpcClient,
) -> anyhow::Result<Vec<(Pubkey, PoolState)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(PoolState::LEN as u64),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(0, PoolState::DISCRIMINATOR.to_vec())),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::finalized()),
            ..Default::default()
        },
        ..Default::default()
    };

    let snapshot = rpc
        .get_program_accounts_with_config(&raydium_cp_swap::id(), config)
        .await?;

    let result = snapshot
        .into_iter()
        .map(|account| {
            let pool: PoolState =
                PoolState::try_deserialize(&mut account.data.as_slice()).unwrap();
            (account.pubkey, pool)
        })
        .collect::<Vec<_>>();

    Ok(result)
}
