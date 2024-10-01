use anchor_lang::AccountDeserialize;
use anchor_spl::token::Mint;
use futures_util::future::join_all;
use itertools::Itertools;
use jsonrpc_core_client::transports::http;
use router_feed_lib::solana_rpc_minimal::rpc_accounts_scan::RpcAccountsScanClient;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::Account;
use solana_sdk::commitment_config::CommitmentConfig;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tracing::{info, trace};

// 4: 388028 mints -> 61 sec
// 16: 388028 mints -> 35 sec
const MAX_PARALLEL_HEAVY_RPC_REQUESTS: usize = 16;

#[derive(Clone, Copy)]
pub struct Token {
    pub mint: Pubkey,
    pub decimals: u8,
}

pub async fn request_mint_metadata(
    rpc_http_url: &str,
    mint_account_ids: &HashSet<Pubkey>,
    max_gma_accounts: usize,
) -> HashMap<Pubkey, Token> {
    info!(
        "Requesting data for mint accounts via chunked gMA for {} pubkey ..",
        mint_account_ids.len()
    );
    let started_at = Instant::now();

    let permits_parallel_rpc_requests = Arc::new(Semaphore::new(MAX_PARALLEL_HEAVY_RPC_REQUESTS));
    let rpc_client = http::connect::<RpcAccountsScanClient>(rpc_http_url)
        .await
        .unwrap();
    let rpc_client = Arc::new(rpc_client);
    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Binary),
        commitment: Some(CommitmentConfig::finalized()),
        data_slice: None,
        min_context_slot: None,
    };

    let mut threads = Vec::new();
    let count = Arc::new(AtomicU64::new(0));
    for pubkey_chunk in mint_account_ids.iter().chunks(max_gma_accounts).into_iter() {
        let pubkey_chunk = pubkey_chunk.into_iter().cloned().collect_vec();
        let count = count.clone();
        let rpc_client = rpc_client.clone();
        let account_ids = pubkey_chunk.iter().map(|x| x.to_string()).collect_vec();
        let account_info_config = account_info_config.clone();
        let permits = permits_parallel_rpc_requests.clone();
        let jh_thread = tokio::spawn(async move {
            let _permit = permits.acquire().await.unwrap();
            let accounts = rpc_client
                .get_multiple_accounts(account_ids.clone(), Some(account_info_config))
                .await
                .unwrap()
                .value;
            let accounts = pubkey_chunk.iter().cloned().zip(accounts).collect_vec();

            let mut mint_accounts: HashMap<Pubkey, Token> = HashMap::with_capacity(accounts.len());
            for (account_pk, ui_account) in accounts {
                if let Some(ui_account) = ui_account {
                    let mut account: Account = ui_account.decode().unwrap();
                    let data = account.data.as_mut_slice();
                    let mint_account = Mint::try_deserialize(&mut &*data).unwrap();
                    trace!(
                        "Mint Account {}: decimals={}",
                        account_pk.to_string(),
                        mint_account.decimals
                    );
                    mint_accounts.insert(
                        account_pk,
                        Token {
                            mint: account_pk,
                            decimals: mint_account.decimals,
                        },
                    );
                    count.fetch_add(1, Ordering::Relaxed);
                }
            }
            mint_accounts
        });
        threads.push(jh_thread);
    } // -- chunks

    let mut merged: HashMap<Pubkey, Token> = HashMap::with_capacity(mint_account_ids.len());
    let maps = join_all(threads).await;
    for map in maps {
        let map = map.expect("thread must succeed");
        merged.extend(map);
    }

    assert_eq!(merged.len() as u64, count.load(Ordering::Relaxed));

    info!(
        "Received {} mint accounts via gMA in {:?}ms",
        count.load(Ordering::Relaxed),
        started_at.elapsed().as_secs_f64() * 1000.0
    );

    merged
}
