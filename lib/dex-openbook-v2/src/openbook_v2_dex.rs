use crate::edge::{load_anchor, OpenbookV2Edge, OpenbookV2EdgeIdentifier};
use crate::openbook_v2_ix_builder;
use anchor_lang::Discriminator;
use anyhow::Context;
use bytemuck::Zeroable;
use itertools::Itertools;
use openbook_v2::state::Market;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode,
    MixedDexSubscription, Quote, SwapInstruction,
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_program::pubkey::Pubkey;
use solana_sdk::clock::Clock;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::sysvar::SysvarId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::{i64, u64};
use tracing::{info, warn};

pub struct OpenbookV2Dex {
    pub edges: HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
}

#[async_trait::async_trait]
impl DexInterface for OpenbookV2Dex {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        _options: HashMap<String, String>,
        enable_compression: bool,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized,
    {
        let markets = fetch_openbook_v2_account(rpc, openbook_v2::id(), enable_compression)
            .await?
            .into_iter()
            .filter(|x| x.1.open_orders_admin.is_none())
            .collect::<Vec<_>>();

        info!("obv2 markets #{}", markets.len());

        let accounts_needed_base = 1 // obv2 program
            + 3 // bids, asks, event heap
            + 4 // market, market auth, base vault, quote vault
            + 1 // out token account
            + super::openbook_v2_ix_builder::INCLUDED_MAKERS_COUNT;

        let edge_pairs = markets
            .iter()
            .map(|(market_pk, market)| {
                let count = |opt: &openbook_v2::pubkey_option::NonZeroPubkeyOption| {
                    if opt.is_some() {
                        1
                    } else {
                        0
                    }
                };
                let accounts_needed = accounts_needed_base
                    + count(&market.open_orders_admin)
                    + count(&market.oracle_a)
                    + count(&market.oracle_b);
                (
                    Arc::new(OpenbookV2EdgeIdentifier {
                        mint_a: market.base_mint,
                        mint_b: market.quote_mint,
                        market: *market_pk,
                        bids: market.bids,
                        asks: market.asks,
                        event_heap: market.event_heap,
                        is_bid: false,
                        account_needed: accounts_needed,
                    }),
                    Arc::new(OpenbookV2EdgeIdentifier {
                        mint_a: market.quote_mint,
                        mint_b: market.base_mint,
                        market: *market_pk,
                        bids: market.bids,
                        asks: market.asks,
                        event_heap: market.event_heap,
                        is_bid: true,
                        account_needed: accounts_needed,
                    }),
                )
            })
            .collect_vec();

        // We want to know what edge needs an update when an account is updated.
        // So create a map from tick_array pks and the whirlpool pk to the target.
        let edges_per_pk = {
            let mut map = HashMap::new();
            for ((market_pk, market), (edge_ask, edge_bid)) in markets.iter().zip(edge_pairs.iter())
            {
                let entry = vec![
                    edge_ask.clone() as Arc<dyn DexEdgeIdentifier>,
                    edge_bid.clone(),
                ];
                map.insert(*market_pk, entry.clone());
                map.insert(market.bids, entry.clone());
                map.insert(market.asks, entry.clone());
                // don't care about the event heap
            }
            map
        };

        Ok(Arc::new(OpenbookV2Dex {
            edges: edges_per_pk,
        }))
    }

    fn name(&self) -> String {
        "OpenbookV2".to_string()
    }

    // NOTE: this is set to program filter, to also capture all open orders
    //       accounts, so that simulation tests don't fail bc. of reading OO
    //       on a different slot than the book side.
    fn subscription_mode(&self) -> DexSubscriptionMode {
        DexSubscriptionMode::Mixed(MixedDexSubscription {
            programs: self.program_ids(),
            accounts: [Clock::id()].into(),
            token_accounts_for_owner: Default::default(),
        })
    }

    fn program_ids(&self) -> HashSet<Pubkey> {
        [openbook_v2::id()].into_iter().collect()
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
            .downcast_ref::<OpenbookV2EdgeIdentifier>()
            .unwrap();

        use openbook_v2::state as o2s;
        let market = load_anchor::<o2s::Market>(chain_data, &id.market)?;
        let bids = load_anchor::<o2s::BookSide>(chain_data, &id.bids).ok();
        let asks = load_anchor::<o2s::BookSide>(chain_data, &id.asks).ok();

        Ok(Arc::new(OpenbookV2Edge { market, bids, asks }))
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
            .downcast_ref::<OpenbookV2EdgeIdentifier>()
            .unwrap();
        let edge = edge.as_any().downcast_ref::<OpenbookV2Edge>().unwrap();

        use openbook_v2::state as o2s;

        #[allow(clippy::clone_on_copy)]
        let mut market = edge.market.clone();

        if edge.bids.is_none() || edge.asks.is_none() {
            warn!("Cant quote {} because missing bid/ask", market.name());
            return Ok(Quote {
                in_amount: 0,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: market.quote_mint,
            });
        }

        let bids = RefCell::new(edge.bids.unwrap());
        let asks = RefCell::new(edge.asks.unwrap());
        let mut event_heap = Box::new({
            let mut heap = o2s::EventHeap::zeroed();
            heap.init();
            heap
        });

        let mut orderbook = o2s::Orderbook {
            bids: bids.borrow_mut(),
            asks: asks.borrow_mut(),
        };

        let clock = chain_data.account(&Clock::id()).context("read clock")?;
        let now_ts = clock.account.deserialize_data::<Clock>()?.unix_timestamp as u64;

        let input_native;
        let order = if id.is_bid {
            let input_lots = in_amount as i64 / market.quote_lot_size;
            input_native = (input_lots * market.quote_lot_size) as u64;
            o2s::Order {
                side: o2s::Side::Bid,
                max_base_lots: market.max_base_lots(),
                max_quote_lots_including_fees: input_lots,
                client_order_id: 0,
                time_in_force: 0,
                params: o2s::OrderParams::Market,
                self_trade_behavior: o2s::SelfTradeBehavior::DecrementTake,
            }
        } else {
            let input_lots = in_amount as i64 / market.base_lot_size;
            input_native = (input_lots * market.base_lot_size) as u64;
            o2s::Order {
                side: o2s::Side::Ask,
                max_base_lots: input_lots,
                max_quote_lots_including_fees: market.max_quote_lots(),
                client_order_id: 0,
                time_in_force: 0,
                params: o2s::OrderParams::Market,
                self_trade_behavior: o2s::SelfTradeBehavior::DecrementTake,
            }
        };

        if input_native == 0 {
            return Ok(Quote {
                in_amount: 0,
                out_amount: 0,
                fee_amount: 0,
                fee_mint: market.quote_mint,
            });
        }

        let result = orderbook.new_order(
            &order,
            &mut market,
            &id.market,
            &mut event_heap,
            None,
            None,
            &Pubkey::default(),
            now_ts,
            10,
            &[],
        )?;

        let out_amount = if id.is_bid {
            result.total_base_taken_native
        } else {
            result.total_quote_taken_native
        };

        Ok(Quote {
            fee_amount: result.taker_fees,
            fee_mint: market.quote_mint,
            in_amount,
            out_amount,
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
            .downcast_ref::<OpenbookV2EdgeIdentifier>()
            .unwrap();
        openbook_v2_ix_builder::build_swap_ix(
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
        Ok(Quote {
            in_amount: u64::MAX,
            out_amount: 0,
            fee_amount: 0,
            fee_mint: Pubkey::default(),
        })
        // let id = id
        //     .as_any()
        //     .downcast_ref::<OpenbookV2EdgeIdentifier>()
        //     .unwrap();
        // let edge = edge.as_any().downcast_ref::<OpenbookV2Edge>().unwrap();

        // use openbook_v2::state as o2s;

        // #[allow(clippy::clone_on_copy)]
        // let mut market = edge.market.clone();

        // if edge.bids.is_none() || edge.asks.is_none() {
        //     warn!("Cant quote {} because missing bid/ask", market.name());
        //     return Ok(Quote {
        //         in_amount: 0,
        //         out_amount: 0,
        //         fee_amount: 0,
        //         fee_mint: market.quote_mint,
        //     });
        // }

        // let bids = RefCell::new(edge.bids.unwrap());
        // let asks = RefCell::new(edge.asks.unwrap());
        // let mut event_heap = Box::new({
        //     let mut heap = o2s::EventHeap::zeroed();
        //     heap.init();
        //     heap
        // });

        // let mut orderbook = o2s::Orderbook {
        //     bids: bids.borrow_mut(),
        //     asks: asks.borrow_mut(),
        // };

        // // TODO: maybe a time service that is primarily based on on-chain time? (can desync!)
        // let now_ts = millis_since_epoch() / 1000;

        // let output_native;
        // let order = if id.is_bid {
        //     let output_lots = out_amount as i64 / market.base_lot_size;
        //     output_native = output_lots * market.base_lot_size;
        //     o2s::Order {
        //         side: o2s::Side::Bid,
        //         max_base_lots: output_native,
        //         max_quote_lots_including_fees: market.max_quote_lots(),
        //         client_order_id: 0,
        //         time_in_force: 0,
        //         params: o2s::OrderParams::Market,
        //         self_trade_behavior: o2s::SelfTradeBehavior::DecrementTake,
        //     }
        // } else {
        //     let output_lots = out_amount as i64 / market.quote_lot_size;
        //     output_native = output_lots * market.quote_lot_size;
        //     o2s::Order {
        //         side: o2s::Side::Ask,
        //         max_base_lots: market.max_base_lots(),
        //         max_quote_lots_including_fees: market.max_quote_lots(),
        //         client_order_id: 0,
        //         time_in_force: 0,
        //         params: o2s::OrderParams::Market,
        //         self_trade_behavior: o2s::SelfTradeBehavior::DecrementTake,
        //     }
        // };

        // if output_native == 0 {
        //     return Ok(Quote {
        //         in_amount: 0,
        //         out_amount: 0,
        //         fee_amount: 0,
        //         fee_mint: market.quote_mint,
        //     });
        // }

        // let result = orderbook.new_order(
        //     &order,
        //     &mut market,
        //     &id.market,
        //     &mut event_heap,
        //     None,
        //     None,
        //     &Pubkey::default(),
        //     now_ts,
        //     10,
        //     &[],
        // )?;

        // let in_amount = if id.is_bid {
        //     result.total_quote_taken_native
        // } else {
        //     result.total_base_taken_native
        // };

        // Ok(Quote {
        //     fee_amount: result.taker_fees,
        //     fee_mint: market.quote_mint,
        //     in_amount,
        //     out_amount,
        // })
    }
}

async fn fetch_openbook_v2_account(
    rpc: &mut RouterRpcClient,
    program_id: Pubkey,
    enable_compression: bool,
) -> anyhow::Result<Vec<(Pubkey, Market)>> {
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            // RpcFilterType::DataSize(8 + size_of::<openbook_v2::state::Market> as u64),
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                0,
                openbook_v2::state::Market::discriminator().to_vec(),
            )),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::finalized()),
            ..Default::default()
        },
        ..Default::default()
    };

    let snapshot = rpc
        .get_program_accounts_with_config(&program_id, config, enable_compression) // todo use compression here
        .await?;

    let result = snapshot
        .iter()
        .map(|account| {
            let market = *bytemuck::from_bytes::<openbook_v2::state::Market>(&account.data[8..]);

            // info!("size: {} vs {}", size_of::<openbook_v2::state::Market>(), account.data().len());
            (account.pubkey, market)
        })
        .collect_vec();

    Ok(result)
}
