use crate::util::{keypair_from_cli, tracing_subscriber_init};
use rand::seq::SliceRandom;
use router_lib::dex::SwapMode;
use router_lib::mango::mango_fetcher::fetch_mango_data;
use router_lib::router_client::RouterClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::RpcClient as BlockingRpcClient;
use solana_client::rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_sdk::account::ReadableAccount;
use solana_sdk::address_lookup_table::state::AddressLookupTable;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signer;
use spl_associated_token_account::get_associated_token_address;
use std::cmp::min;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::str::FromStr;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

mod config;
mod util;

struct Bot {
    rpc_client: RpcClient,
    outgoing_rpc_client: RpcClient,
    swap_client: RouterClient,
    wallet: Keypair,
    min_sol: u64,
    blocking_rpc_client: BlockingRpcClient,
}

impl Bot {
    pub fn sol() -> Pubkey {
        Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap()
    }

    pub fn usdc() -> Pubkey {
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap()
    }

    pub async fn swap(&self, from: &Pubkey, to: &Pubkey, mut amount: u64) -> anyhow::Result<u64> {
        let max_attempt = 4;
        for i in 1..max_attempt + 1 {
            if *from == Bot::sol() {
                let balance = self.balance(from).await?;
                if balance <= self.min_sol {
                    return Ok(0);
                }

                let max_amount = balance - self.min_sol;
                amount = min(max_amount, amount);
            } else {
                let balance_from = self.balance(from).await?;
                amount = min(balance_from, amount);
            }

            info!("swap {} {} => {}", amount, from, to);
            let balance_before = self.balance(to).await?;

            match self.swap_internal(from, to, amount).await {
                Ok(_) => {}
                Err(e) => {
                    if i == max_attempt {
                        anyhow::bail!("failed to swap: {}", e);
                    }

                    let duration_secs = (i as f64 * 3.0 * 1.2_f64.powi(i)).ceil() as u64;
                    warn!(
                        "swap failed with error: {} (sleeping for {} before retrying)",
                        e, duration_secs
                    );
                    sleep(Duration::from_secs(duration_secs)).await;
                    continue;
                }
            };

            info!("swap confirmed");

            let balance_after = self.balance(to).await?;

            // if mint is sol, can actually decrease because of fees and ATA creation
            if balance_after < balance_before && *to == Bot::sol() {
                return Ok(0);
            } else {
                return Ok(balance_after - balance_before);
            }
        }

        anyhow::bail!("Failed to swap")
    }

    async fn swap_internal(&self, from: &Pubkey, to: &Pubkey, amount: u64) -> anyhow::Result<()> {
        let quote = self
            .swap_client
            .quote(*from, *to, amount, 50, false, 28, SwapMode::ExactIn)
            .await?;

        info!(
            "quote {} {} => {} {}",
            quote.in_amount.clone().expect("in amount"),
            from,
            quote.out_amount,
            to
        );
        debug!("{:?}", quote.clone());

        let load_alt = |alt_addr| {
            let alt_data = self.blocking_rpc_client.get_account(&alt_addr);

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

        let quote_slot = quote.context_slot;
        let (latest_blockhash, _) = self
            .outgoing_rpc_client
            .get_latest_blockhash_with_commitment(CommitmentConfig::finalized())
            .await?;
        let latest_slot = self
            .outgoing_rpc_client
            .get_slot_with_commitment(CommitmentConfig::processed())
            .await?;
        let tx = self
            .swap_client
            .swap(load_alt, quote.clone(), &self.wallet, latest_blockhash)
            .await?;

        info!(
            "swap sig: {} / quote slot: {} / latest_slot: {}",
            tx.signatures[0], quote_slot, latest_slot
        );

        if let Some(router_accounts) = quote.accounts {
            let simulation_result = self
                .outgoing_rpc_client
                .simulate_transaction_with_config(
                    &tx,
                    RpcSimulateTransactionConfig {
                        sig_verify: false,
                        replace_recent_blockhash: false,
                        commitment: Some(CommitmentConfig::processed()),
                        encoding: None,
                        accounts: None,
                        min_context_slot: None,
                    },
                )
                .await;
            match simulation_result {
                Ok(s) => {
                    if s.value.err.is_some() {
                        warn!("Simulation failed! {:?}", s.value.err.unwrap());

                        let addresses = router_accounts
                            .iter()
                            .map(|x| Pubkey::from_str(x.address.as_str()).unwrap())
                            .collect::<Vec<_>>();
                        let rpc_accounts = self
                            .outgoing_rpc_client
                            .get_multiple_accounts_with_commitment(
                                &addresses,
                                CommitmentConfig::processed(),
                            )
                            .await?;

                        warn!(
                            "- Has rpc_accounts ?: {} (slot={})",
                            rpc_accounts.value.len(),
                            rpc_accounts.context.slot
                        );
                        warn!("- Has router_accounts ?: {}", router_accounts.len());

                        for (rpc_account, router_account) in
                            rpc_accounts.value.iter().zip(router_accounts.iter())
                        {
                            let Some(rpc_account) = rpc_account else {
                                warn!(" - empty for {}", router_account.address);
                                continue;
                            };

                            if rpc_account.data() != router_account.data.as_slice() {
                                warn!(
                                    "- Difference for account: {}, slot={}",
                                    router_account.address, router_account.slot
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Simulation error! {:?}", e);
                }
            }
        }

        self.outgoing_rpc_client
            .send_and_confirm_transaction_with_spinner_and_config(
                &tx,
                CommitmentConfig::confirmed(),
                RpcSendTransactionConfig {
                    skip_preflight: true,
                    preflight_commitment: Some(CommitmentLevel::Confirmed),
                    ..RpcSendTransactionConfig::default()
                },
            )
            .await?;

        Ok(())
    }

    pub async fn balance(&self, mint: &Pubkey) -> anyhow::Result<u64> {
        let max_attempt = 4;
        for i in 1..max_attempt + 1 {
            match self.balance_internal(mint).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    if i == max_attempt {
                        break;
                    }

                    let duration_secs = (i as f64 * 3.0 * 1.2_f64.powi(i)).ceil() as u64;
                    warn!(
                        "failed to retrieve balance: {} (sleeping for {} before retrying)",
                        e, duration_secs
                    );
                    sleep(Duration::from_secs(duration_secs)).await;
                }
            }
        }

        anyhow::bail!("failed to retrieve balance (RPC issue ?)");
    }

    pub async fn balance_internal(&self, mint: &Pubkey) -> anyhow::Result<u64> {
        if *mint == Bot::sol() {
            let balance = self.rpc_client.get_balance(&self.wallet.pubkey()).await?;
            debug!("balance of SOL is {}", balance);
            return Ok(balance);
        }

        let ata = get_associated_token_address(&self.wallet.pubkey(), mint);
        let account_res = self.rpc_client.get_account(&ata).await;

        match account_res {
            Ok(account) => {
                let balance_data = &account.data[64..(64 + 8)];
                let balance = u64::from_le_bytes(balance_data.try_into().unwrap());

                debug!("balance of {} is {}", mint, balance);

                Ok(balance)
            }
            Err(e) => {
                error!("failed to retrieve balance of {} ({})", mint, e);
                Ok(0)
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber_init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Please enter a config file path argument.");
        return Ok(());
    }

    let config: config::Config = {
        let mut file = File::open(&args[1])?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        toml::from_str(&contents).unwrap()
    };

    let mints: HashSet<_> = config
        .mints
        .iter()
        .map(|x| Pubkey::from_str(x).unwrap())
        .collect();
    let mints = if config.use_mango_tokens {
        let mango_mints = fetch_mango_data().await?;
        mints
            .into_iter()
            .chain(mango_mints.mints.into_iter())
            .collect::<HashSet<_>>()
    } else {
        mints
    };

    info!("Using {} mints", mints.len());

    let wallet = keypair_from_cli(config.owner.as_str());
    let rpc_client =
        RpcClient::new_with_commitment(config.rpc_http_url.clone(), CommitmentConfig::confirmed());
    let outgoing_rpc_client = RpcClient::new_with_commitment(
        config.outgoing_rpc_http_url.clone(),
        CommitmentConfig::confirmed(),
    );
    let blocking_rpc_client =
        BlockingRpcClient::new_with_commitment(config.rpc_http_url, CommitmentConfig::confirmed());
    let swap_client = RouterClient {
        http_client: reqwest::Client::builder().build()?,
        router_url: config.router,
    };
    let mut interval = tokio::time::interval(Duration::from_secs(config.execution_interval_sec));

    let min_sol = 100_000_000; // 0.1 SOL
    let bot = Bot {
        wallet,
        rpc_client,
        outgoing_rpc_client,
        blocking_rpc_client,
        swap_client,
        min_sol,
    };

    let usdc = Bot::usdc();
    let sol = Bot::sol();

    // Step 1 - Move all to USDC (except min SOL for gas)
    info!("#1 --- Startup ---");
    for mint in &mints {
        let balance = bot.balance(mint).await.expect("failed to get balance");

        if balance > 100
        /* dust */
        {
            if *mint == sol {
                info!(
                    "Startup balance: {} for {}; will keep at least {}",
                    balance, mint, bot.min_sol
                );
            } else {
                info!("Startup balance: {} for {}", balance, mint);
            }

            if *mint == usdc {
                continue;
            }

            match bot.swap(mint, &usdc, balance).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Rebalancing swap failed: {:?}", e)
                }
            }
        }
    }

    // Step 2 - Random swaps
    // - USDC => A
    // - A => B
    // - B => USDC
    loop {
        // refill SOL if needed
        let mut sol_balance = bot.balance(&sol).await?;
        while sol_balance < bot.min_sol {
            info!("## --- Refill - USDC => SOL ---");
            bot.swap(&usdc, &sol, 1_000_000) // 1$
                .await
                .expect("Refill swap failed");
            sol_balance = bot.balance(&sol).await?;
        }

        info!("#2 --- USDC => X => Y => USDC ---");
        let amount = *config.amounts.choose(&mut rand::thread_rng()).unwrap();
        let mut tokens = mints.iter().filter(|x| **x != usdc).collect::<Vec<_>>();
        tokens.shuffle(&mut rand::thread_rng());

        let amount = bot
            .swap(&usdc, tokens[0], amount)
            .await
            .expect("First swap failed");

        info!("---");
        let amount = bot
            .swap(tokens[0], tokens[1], amount)
            .await
            .expect("Second swap failed");

        info!("---");
        let _amount = bot
            .swap(tokens[1], &usdc, amount)
            .await
            .expect("Third swap failed");

        interval.tick().await;
    }
}
