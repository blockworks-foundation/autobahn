use crate::cli_args::{Cli, Command};
use crate::util::{string_or_env, tracing_subscriber_init};
use autobahn_executor::logs::*;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use bincode::serialize;
use clap::Parser;
use router_config_lib::Config;
use router_lib::dex::SwapMode;
use router_lib::router_client::RouterClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::RpcClient as BlockingRpcClient;
use solana_client::rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_sdk::address_lookup_table::state::AddressLookupTable;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::EncodableKey;
use std::str::FromStr;

mod cli_args;
mod util;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber_init();
    let cli = Cli::parse();

    match cli.command {
        Command::Swap(swap) => {
            let wallet = Keypair::read_from_file(swap.owner).expect("couldn't read keypair");
            let rpc_client = RpcClient::new(string_or_env(swap.rpc.url.clone()));
            let blocking_rpc_client = BlockingRpcClient::new(string_or_env(swap.rpc.url));
            let swap_client = RouterClient {
                http_client: reqwest::Client::builder().build()?,
                router_url: string_or_env(swap.router.to_string()),
            };

            let Ok(swap_mode) = SwapMode::from_str(&swap.swap_mode) else {
                anyhow::bail!("Swap mode should be either ExactIn(default) or ExactOut");
            };

            let quote = swap_client
                .quote(
                    Pubkey::from_str(&swap.input_mint).unwrap(),
                    Pubkey::from_str(&swap.output_mint).unwrap(),
                    swap.amount,
                    swap.slippage_bps,
                    false,
                    40,
                    swap_mode,
                )
                .await?;

            println!("quote: {:?}", quote);

            let (latest_blockhash, _) = rpc_client
                .get_latest_blockhash_with_commitment(CommitmentConfig::finalized())
                .await?;

            let load_alt = |alt_addr| {
                let alt_data = blocking_rpc_client.get_account(&alt_addr);

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

            let tx = swap_client
                .swap(load_alt, quote, &wallet, latest_blockhash)
                .await?;

            let sim = rpc_client
                .simulate_transaction_with_config(
                    &tx,
                    RpcSimulateTransactionConfig {
                        commitment: Some(CommitmentConfig::processed()),
                        sig_verify: false,
                        replace_recent_blockhash: true,
                        encoding: None,
                        accounts: None,
                        min_context_slot: None,
                    },
                )
                .await?;

            println!("sim swap: err={:?}", sim.value.err);
            if let Some(logs) = sim.value.logs {
                for log in logs.iter() {
                    println!("{}", log);
                }
            }

            let binary = serialize(&tx)?;
            let base = BASE64_STANDARD.encode(binary);
            println!(
                "inspect: got to https://explorer.solana.com/tx/inspector and paste {}",
                base
            );

            let sig = rpc_client
                .send_and_confirm_transaction_with_spinner_and_config(
                    &tx,
                    CommitmentConfig::processed(),
                    RpcSendTransactionConfig {
                        skip_preflight: true,
                        ..RpcSendTransactionConfig::default()
                    },
                )
                .await?;

            println!("swap success: {}", sig);
        }
        Command::Quote(quote) => {
            let Ok(swap_mode) = SwapMode::from_str(&quote.swap_mode) else {
                anyhow::bail!("Swap mode should be either ExactIn(default) or ExactOut");
            };

            let swap_client = RouterClient {
                http_client: reqwest::Client::builder().build()?,
                router_url: string_or_env(quote.router.to_string()),
            };

            let quote = swap_client
                .quote(
                    Pubkey::from_str(&quote.input_mint).unwrap(),
                    Pubkey::from_str(&quote.output_mint).unwrap(),
                    quote.amount,
                    quote.slippage_bps,
                    false,
                    40,
                    swap_mode,
                )
                .await?;

            println!("quote: {:?}", quote);
        }
        Command::DownloadTestPrograms(download) => {
            let _config = Config::load(&download.config)?;
        }
        Command::DecodeLog(log) => {
            let decoded = BASE64_STANDARD.decode(log.data)?;
            let discriminant: &[u8; 8] = &decoded[..8].try_into().unwrap();
            match discriminant {
                &SWAP_EVENT_DISCRIMINANT => {
                    let event = bytemuck::from_bytes::<SwapEvent>(&decoded[8..]);
                    println!("SwapEvent - input_amount: {}, input_mint: {:?}, output_amount: {}, output_mint: {:?}", event.input_amount, event.input_mint, event.output_amount, event.output_mint);
                }
                &PLATFORM_FEE_LOG_DISCRIMINANT => {
                    let event = bytemuck::from_bytes::<PlatformFeeLog>(&decoded[8..]);
                    println!("PlatformFeeLog - user: {:?}, platform_token_account: {:?}, platform_fee: {}", event.user, event.platform_token_account, event.platform_fee);
                }
                &REFERRER_FEE_LOG_DISCRIMINANT => {
                    let event = bytemuck::from_bytes::<ReferrerFeeLog>(&decoded[8..]);
                    println!("ReferrerFeeLog - referree: {:?}, referer_token_account: {:?}, referrer_fee: {}", event.referee, event.referer_token_account, event.referrer_fee);
                }
                &REFERRER_WITHDRAW_LOG_DISCRIMINANT => {
                    let event = bytemuck::from_bytes::<ReferrerWithdrawLog>(&decoded[8..]);
                    println!("ReferrerWithdrawLog - referer: {:?}, referer_token_account: {:?}, amount: {}", event.referer, event.referer_token_account, event.amount);
                }
                &CREATE_REFERRAL_LOG_DISCRIMINANT => {
                    let event = bytemuck::from_bytes::<CreateReferralLog>(&decoded[8..]);
                    println!("CreateReferralLog - referer: {:?}, referee: {:?}, vault: {:?}, mint: {:?}", event.referer, event.referee, event.vault, event.mint);
                }
                _ => panic!("Unknown log discriminant"),
            }
        }
    }

    Ok(())
}
