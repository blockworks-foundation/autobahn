use crate::config::Config;
use crate::persister::PersistableState;
use async_channel::Sender;
use itertools::{iproduct, Itertools};
use rand::seq::SliceRandom;
use router_config_lib::PriceFeedConfig;
use router_lib::dex::SwapMode;
use router_lib::mango::mango_fetcher::fetch_mango_data;
use router_lib::model::quote_response::QuoteResponse;
use router_lib::price_feeds::composite::CompositePriceFeed;
use router_lib::price_feeds::price_cache::PriceCache;
use router_lib::price_feeds::price_feed::PriceFeed;
use router_lib::router_client::RouterClient;
use solana_client::client_error::reqwest;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::RpcClient as BlockingRpcClient;
use solana_client::rpc_config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_program::address_lookup_table::state::AddressLookupTable;
use solana_program::address_lookup_table::AddressLookupTableAccount;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::{Account, ReadableAccount};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account::get_associated_token_address;
use spl_token::state::Mint;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

pub(crate) async fn run(
    config: &Config,
    sender: Sender<PersistableState>,
    mut exit: Receiver<()>,
) -> anyhow::Result<()> {
    let mut mints = get_mints(config).await?.into_iter().collect_vec();
    mints.shuffle(&mut rand::thread_rng());

    info!(count = mints.len(), "Running with mints");

    let (mut price_feed, _pf_job) = CompositePriceFeed::start(
        PriceFeedConfig {
            birdeye_token: config.birdeye_token.to_string(),
            birdeye_single_mode: None,
            refresh_interval_secs: 25,
        },
        exit.resubscribe(),
    );
    let (price_cache, _pc_job) = PriceCache::new(exit.resubscribe(), price_feed.receiver());
    for m in &mints {
        price_feed.register_mint_sender().send(*m).await?;
    }

    let rpc_client =
        RpcClient::new_with_commitment(config.rpc_http_url.clone(), CommitmentConfig::confirmed());
    let mints_accounts = rpc_client
        .get_multiple_accounts(mints.iter().copied().collect_vec().as_slice())
        .await?
        .into_iter()
        .zip(&mints)
        .filter_map(|(account, key)| {
            if let Some(acc) = account {
                let mint_acc = Mint::unpack(acc.data().iter().as_ref()).ok();
                mint_acc.map(|ma| (*key, ma))
            } else {
                None
            }
        })
        .collect::<HashMap<Pubkey, Mint>>();

    let mut interval = tokio::time::interval(Duration::from_secs(config.execution_interval_sec));

    let usdc = Bot::usdc();
    let sol = Bot::sol();

    let router_bot = build_bot(config, config.router.clone())?;
    let jupiter_bot = build_bot(config, config.jupiter.clone())?;

    let other_tokens = vec![usdc, sol];

    let amounts = &config.amounts;
    let max_accounts = [30, 40, 60_usize];

    let test_cases = iproduct!(other_tokens, mints, amounts, max_accounts).collect_vec();
    info!(count = test_cases.len(), "Test cases");

    let mut test_cases_index = 0;

    loop {
        tokio::select! {
            _ = exit.recv() => {
                warn!("shutting down persister...");
                break;
            },
            _ = interval.tick() => {
                if test_cases_index >= test_cases.len() {
                    test_cases_index = 0;
                }

                let test_case = &test_cases[test_cases_index];

                let from_token = test_case.0;
                let to_token = test_case.1;

                if from_token == to_token {
                    test_cases_index += 1;
                    continue;
                }

                let amount_dollar = *test_case.2;
                let max_account = test_case.3;

                let Some(price_ui) = price_cache.price_ui(from_token) else {
                    test_cases_index += 1;
                    continue;
                };
                let Some(decimals) = mints_accounts.get(&from_token) else {
                    test_cases_index += 1;
                    continue;
                };
                let Some(out_price_ui) = price_cache.price_ui(to_token) else {
                    test_cases_index += 1;
                    continue;
                };
                let Some(out_decimals) = mints_accounts.get(&to_token) else {
                    test_cases_index += 1;
                    continue;
                };

                let multiplier = 10_u32.pow(decimals.decimals as u32) as f64;
                let amount_native =
                        ((amount_dollar as f64 / price_ui) * multiplier).round() as u64;

                let out_multiplier = 10_u32.pow(out_decimals.decimals as u32) as f64;
                let out_fx_dollar = out_price_ui / out_multiplier;

                info!(%from_token, %to_token, amount_dollar, amount_native, price_ui, out_price_ui, max_account, "Running test on");

                let sndr = sender.clone();
                let r_bot = router_bot.clone();
                let j_bot = jupiter_bot.clone();

                tokio::spawn(async move {
                    match simulate(&from_token, &to_token, amount_native, amount_dollar as f64, out_fx_dollar, r_bot, j_bot, max_account).await {
                        Ok(state) => {sndr.send(state).await.expect("sending state must succeed");}
                        Err(e) => { error!("failed to simulate: {:?}", e)}
                    }
                });

                test_cases_index += 1;
            }
        }
    }

    Ok(())
}

fn build_bot(config: &Config, url: String) -> anyhow::Result<Arc<Bot>> {
    let outgoing_rpc_client = RpcClient::new_with_commitment(
        config.outgoing_rpc_http_url.clone(),
        CommitmentConfig::processed(),
    );
    let blocking_rpc_client = BlockingRpcClient::new_with_commitment(
        config.rpc_http_url.clone(),
        CommitmentConfig::processed(),
    );
    let swap_client = RouterClient {
        http_client: reqwest::Client::builder().build()?,
        router_url: url,
    };

    let bot = Bot {
        wallet: Pubkey::from_str(config.wallet_pubkey.as_str()).unwrap(),
        outgoing_rpc_client,
        blocking_rpc_client,
        swap_client,
    };

    Ok(Arc::new(bot))
}

#[allow(clippy::too_many_arguments)]
async fn simulate(
    from: &Pubkey,
    to: &Pubkey,
    amount: u64,
    amount_dollars: f64,
    out_fx_dollars: f64,
    router_bot: Arc<Bot>,
    jupiter_bot: Arc<Bot>,
    max_accounts: usize,
) -> anyhow::Result<PersistableState> {
    let bot = router_bot.clone();
    let load_alt = |alt_addr| {
        let alt_data = bot.blocking_rpc_client.get_account(&alt_addr);

        match alt_data {
            Ok(alt_data) => Some(AddressLookupTableAccount {
                key: alt_addr,
                addresses: AddressLookupTable::deserialize(alt_data.data.as_slice())
                    .unwrap()
                    .addresses
                    .to_vec(),
            }),
            Err(_) => None,
        }
    };

    let from = *from;
    let to = *to;

    // quote jup + autobahn-router
    let bot = router_bot.clone();
    let router = tokio::spawn(async move {
        bot.swap_client
            .quote(from, to, amount, 50, false, max_accounts, SwapMode::ExactIn)
            .await
    });
    let bot = jupiter_bot.clone();
    let jupiter = tokio::spawn(async move {
        bot.swap_client
            .quote(from, to, amount, 50, false, max_accounts, SwapMode::ExactIn)
            .await
    });

    let router = router.await?;
    let jupiter = jupiter.await?;
    let router_route = build_route(&router);
    let jupiter_route = build_route(&jupiter);

    // wait
    sleep(Duration::from_secs(5)).await;

    // simulate
    let router_result = simulate_swap(router_bot, load_alt, router, "autobahn").await?;
    let jupiter_result = simulate_swap(jupiter_bot, load_alt, jupiter, "jupiter").await?;

    Ok(PersistableState {
        input_mint: from,
        output_mint: to,
        input_amount: amount,
        input_amount_in_dollars: amount_dollars,
        max_accounts,
        router_quote_output_amount: router_result.0,
        jupiter_quote_output_amount: jupiter_result.0,
        router_simulation_is_success: router_result.1,
        jupiter_simulation_is_success: jupiter_result.1,
        router_accounts: router_result.2,
        jupiter_accounts: jupiter_result.2,
        router_output_amount_in_dollars: router_result.0 as f64 * out_fx_dollars,
        jupiter_output_amount_in_dollars: jupiter_result.0 as f64 * out_fx_dollars,
        router_route,
        jupiter_route,
        router_actual_output_amount: router_result.3,
        jupiter_actual_output_amount: jupiter_result.3,
        router_error: router_result.4,
        jupiter_error: jupiter_result.4,
    })
}

fn build_route(quote: &anyhow::Result<QuoteResponse>) -> String {
    match quote {
        Ok(quote) => {
            let steps = quote
                .route_plan
                .iter()
                .filter_map(|x| x.swap_info.clone().map(|step| step.label))
                .collect::<Vec<Option<String>>>();
            let steps = steps
                .into_iter()
                .map(|x| x.unwrap_or("?".to_string()))
                .collect::<HashSet<_>>();
            steps.iter().join(", ")
        }
        Err(_) => "".to_string(),
    }
}

async fn simulate_swap<F>(
    bot: Arc<Bot>,
    alt: F,
    quote: anyhow::Result<QuoteResponse>,
    name: &str,
) -> anyhow::Result<(u64, bool, usize, u64, String)>
where
    F: Fn(Pubkey) -> Option<AddressLookupTableAccount>,
{
    let Ok(quote) = quote else {
        let err = quote.unwrap_err();
        warn!("Quote failed for {} with {:?}", name, err);
        return Ok((0, false, 0, 0, parse_error(err.to_string())));
    };

    let latest_blockhash = bot
        .outgoing_rpc_client
        .get_latest_blockhash_with_commitment(CommitmentConfig::finalized())
        .await?
        .0;
    let out_amount = u64::from_str(quote.out_amount.as_str())?;

    let tx = bot
        .swap_client
        .simulate_swap(alt, quote.clone(), &bot.wallet, latest_blockhash, true)
        .await;

    let Ok(tx) = tx else {
        warn!("Failed to build TX for {}: {:?}", name, tx.unwrap_err());
        return Ok((out_amount, false, 0, 0, "failed to build TX".to_string()));
    };

    let accounts = count_account(&tx);
    let out_token_account = get_ata(
        bot.wallet,
        Pubkey::from_str(quote.output_mint.as_str()).unwrap(),
    );

    let initial_balance = bot
        .blocking_rpc_client
        .get_account(&out_token_account)
        .ok()
        .map(|x| get_balance(x).ok())
        .flatten();

    let simulation_result = bot.blocking_rpc_client.simulate_transaction_with_config(
        &tx,
        RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: false,
            commitment: Some(CommitmentConfig::processed()),
            encoding: None,
            accounts: Some(RpcSimulateTransactionAccountsConfig {
                encoding: None,
                addresses: vec![out_token_account.to_string()],
            }),
            min_context_slot: None,
        },
    );
    let Ok(simulation_result) = simulation_result else {
        let bytes = bincode::serialize(&tx).unwrap();
        let lut_used = tx
            .message
            .address_table_lookups()
            .map(|x| x.len())
            .unwrap_or(0);

        warn!(
            "Failed to simulate TX for {}: (size={}, nb_lut={}) {:?}",
            name,
            bytes.len(),
            lut_used,
            simulation_result.unwrap_err()
        );
        return Ok((
            out_amount,
            false,
            accounts,
            0,
            "failed to simulate TX".to_string(),
        ));
    };

    if let Some(err) = simulation_result.value.err {
        warn!("Tx failed for {}: {:?}", name, err);

        let mut is_slippage_error = false;
        let mut is_cu_error = false;

        if let Some(logs) = simulation_result.value.logs {
            for l in &logs {
                warn!(" - {}", l);
                if l.contains("AmountOutBelowMinimum") {
                    is_slippage_error = true;
                }
                if l.contains("Max slippage reached") {
                    is_slippage_error = true;
                }
                if l.contains("exceeded CUs meter at BPF") {
                    is_cu_error = true;
                }
            }
        }

        let err_str = if is_slippage_error {
            "Failed to execute TX : Max Slippage Reached".to_string()
        } else if is_cu_error {
            "Failed to execute TX : Exceeded CUs meter".to_string()
        } else {
            format!("Failed to execute TX : {err:?}")
        };

        return Ok((out_amount, false, accounts, 0, err_str));
    };

    let Some(after_accounts) = simulation_result.value.accounts else {
        warn!("Tx success for {}: but missing accounts", name);
        return Ok((
            out_amount,
            false,
            accounts,
            0,
            "Missing simulation accounts".to_string(),
        ));
    };

    let Some(after_account) = after_accounts.into_iter().next().flatten() else {
        warn!("Tx success for {}: but missing account", name);
        return Ok((
            out_amount,
            false,
            accounts,
            0,
            "Missing simulation account".to_string(),
        ));
    };

    let Some(after_account) = after_account.decode::<Account>() else {
        warn!("Tx success for {}: but failed to decode account", name);
        return Ok((
            out_amount,
            false,
            accounts,
            0,
            "Failed to decode account".to_string(),
        ));
    };

    let final_balance = get_balance(after_account);
    let actual_amount = final_balance.unwrap_or(0) - initial_balance.unwrap_or(0);

    info!("Tx success for {} with {} accounts", name, accounts);
    Ok((out_amount, true, accounts, actual_amount, "".to_string()))
}

fn parse_error(err: String) -> String {
    if err.contains("no path between") {
        "no path found".to_string()
    } else if err.contains("bad route") {
        "bad route".to_string()
    } else {
        err
    }
}

fn get_ata(wallet: Pubkey, mint: Pubkey) -> Pubkey {
    get_associated_token_address(&wallet, &mint)
}

fn get_balance(account: Account) -> anyhow::Result<u64> {
    Ok(spl_token::state::Account::unpack(account.data.as_slice())?.amount)
}

fn count_account(tx: &VersionedTransaction) -> usize {
    tx.message.static_account_keys().len()
        + tx.message
            .address_table_lookups()
            .map(|x| x.len())
            .unwrap_or(0)
}

async fn get_mints(config: &Config) -> anyhow::Result<HashSet<Pubkey>> {
    let configured_mints: HashSet<_> = config
        .mints
        .iter()
        .map(|x| Pubkey::from_str(x).unwrap())
        .collect();

    let mints = if config.use_mango_tokens {
        let mango_mints = fetch_mango_data().await?;
        configured_mints
            .into_iter()
            .chain(mango_mints.mints.into_iter())
            .collect::<HashSet<_>>()
    } else {
        configured_mints
    };

    Ok(mints)
}

struct Bot {
    outgoing_rpc_client: RpcClient,
    swap_client: RouterClient,
    wallet: Pubkey,
    blocking_rpc_client: BlockingRpcClient,
}

impl Bot {
    pub fn sol() -> Pubkey {
        Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap()
    }

    pub fn usdc() -> Pubkey {
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap()
    }
}
