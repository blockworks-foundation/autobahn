use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use jupiter_amm_interface::{Amm, QuoteParams, SwapMode};
use s_jup_interface::{SPoolInitAccounts, SPoolInitKeys, SPoolJup};
use s_sol_val_calc_prog_aggregate::{LstSolValCalc, MutableLstSolValCalc};
use sanctum_lst_list::{
    inf_s_program, lido_program, marinade_program, sanctum_spl_multi_stake_pool_program,
    sanctum_spl_stake_pool_program, spl_stake_pool_program, SanctumLstList,
};
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;
use solana_sdk_macro::pubkey;

use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode, Quote,
    SwapInstruction,
};

use crate::edge::{InfinityEdge, InfinityEdgeIdentifier};
use crate::ix_builder;

pub const INF_LP_PK: Pubkey = pubkey!("5oVNBeEEQvYi1cX3ir8Dx5n1P7pdxydbGF2X4TxVusJm");

pub struct InfinityDex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
    pub subscribed_pks: HashSet<Pubkey>,
    pub programs: Vec<(Pubkey, String)>,
}

#[async_trait::async_trait]
impl DexInterface for InfinityDex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let program_id = s_controller_lib::program::ID;
        let SanctumLstList { sanctum_lst_list } = SanctumLstList::load();

        let SPoolInitKeys {
            lst_state_list,
            pool_state,
        } = SPoolJup::init_keys(program_id);
        let lst_state_list_account = rpc.get_account(&lst_state_list).await.unwrap();
        let pool_state_account = rpc.get_account(&pool_state).await.unwrap();

        let amm: s_jup_interface::SPool<Account, Account> = SPoolJup::from_init_accounts(
            program_id,
            SPoolInitAccounts {
                lst_state_list: lst_state_list_account.clone(),
                pool_state: pool_state_account.clone(),
            },
            &sanctum_lst_list,
        )?;

        let subscribed_pks =
            HashSet::<Pubkey>::from_iter(amm.get_accounts_to_update_full().iter().copied());

        let mut edges_per_pk: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> = HashMap::new();

        for lst_data in amm.lst_data_list.iter().flatten() {
            let lst_mint = lst_data.sol_val_calc.lst_mint();
            let account_metas = lst_data.sol_val_calc.ix_accounts();
            let num_accounts_for_tx = account_metas.len();
            let Ok((lst_state, lst_data)) = amm.find_ready_lst(lst_mint) else {
                continue;
            };

            if lst_state.is_input_disabled != 0 {
                continue;
            }

            for pk in lst_data.sol_val_calc.get_accounts_to_update() {
                let edges = vec![
                    Arc::new(InfinityEdgeIdentifier {
                        input_mint: INF_LP_PK,
                        output_mint: lst_mint,
                        accounts_needed: 10 + num_accounts_for_tx,
                        is_output_lp: true,
                    }) as Arc<dyn DexEdgeIdentifier>,
                    Arc::new(InfinityEdgeIdentifier {
                        input_mint: lst_mint,
                        output_mint: INF_LP_PK,
                        accounts_needed: 10 + num_accounts_for_tx,
                        is_output_lp: false,
                    }),
                ];

                if let Some(edges_per_pk) = edges_per_pk.get_mut(&pk) {
                    edges_per_pk.extend(edges.iter().cloned());
                } else {
                    edges_per_pk.insert(pk, edges);
                }
            }
        }

        let programs = amm.program_dependencies();

        // TODO Why is there more subscribed than in the update map ?

        let dex = InfinityDex {
            edges: edges_per_pk,
            subscribed_pks,
            programs,
        };

        Ok(Arc::new(dex))
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [
            Pubkey::from_str("5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx").unwrap(),
            s_controller_lib::program::ID,
            sanctum_spl_multi_stake_pool_program::ID,
            sanctum_spl_stake_pool_program::ID,
            lido_program::ID,
            marinade_program::ID,
            inf_s_program::ID,
            flat_fee_interface::ID,
            spl_stake_pool_program::ID,
        ]
        .into_iter()
        .chain(self.programs.iter().map(|x| x.0))
        .collect()
    }

    fn name(&self) -> String {
        "Infinity".to_string()
    }

    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Accounts(
            self.edges
                .keys()
                .cloned()
                .chain(self.subscribed_pks.iter().cloned())
                .collect(),
        )
    }

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
        self.edges.clone()
    }

    fn load(
        &self,
        _id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>> {
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

        return Ok(Arc::new(InfinityEdge { data: amm }));
    }

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        _chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        let id = id
            .as_any()
            .downcast_ref::<InfinityEdgeIdentifier>()
            .unwrap();
        let edge = edge.as_any().downcast_ref::<InfinityEdge>().unwrap();

        let (input_mint, output_mint) = (id.input_mint, id.output_mint);

        let quote = edge.data.quote(&QuoteParams {
            amount: in_amount,
            input_mint,
            output_mint,
            swap_mode: SwapMode::ExactIn,
        })?;

        let out_amount = if quote.not_enough_liquidity {
            0
        } else {
            quote.out_amount
        };

        Ok(Quote {
            in_amount,
            out_amount,
            fee_amount: quote.fee_amount,
            fee_mint: quote.fee_mint,
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
            .downcast_ref::<InfinityEdgeIdentifier>()
            .unwrap();
        ix_builder::build_swap_ix(
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
        let id = id
            .as_any()
            .downcast_ref::<InfinityEdgeIdentifier>()
            .unwrap();
        let edge = edge.as_any().downcast_ref::<InfinityEdge>().unwrap();

        let (input_mint, output_mint) = (id.input_mint, id.output_mint);

        let quote = edge.data.quote(&QuoteParams {
            amount: out_amount,
            input_mint,
            output_mint,
            swap_mode: SwapMode::ExactOut,
        })?;

        let in_amount = if quote.not_enough_liquidity {
            u64::MAX
        } else {
            quote.in_amount
        };

        Ok(Quote {
            in_amount,
            out_amount,
            fee_amount: quote.fee_amount,
            fee_mint: quote.fee_mint,
        })
    }
}
