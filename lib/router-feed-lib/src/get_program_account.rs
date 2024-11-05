use std::collections::HashSet;
use std::{str::FromStr, sync::Arc, time::Duration};

use crate::account_write::{account_write_from, AccountWrite, SNAP_ACCOUNT_WRITE_VERSION};
use crate::solana_rpc_minimal::rpc_accounts_scan::RpcAccountsScanClient;
use base64::Engine;
use jsonrpc_core_client::transports::http;
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_response::OptionalContext,
};
use solana_rpc_client_api::filter::{Memcmp, RpcFilterType};
use solana_sdk::account::Account;
use solana_sdk::{account::AccountSharedData, commitment_config::CommitmentConfig, pubkey::Pubkey};
use tracing::info;

pub struct CustomSnapshotProgramAccounts {
    pub slot: u64,
    pub program_id: Option<Pubkey>,
    pub accounts: Vec<AccountWrite>,
    pub missing_accounts: Vec<Pubkey>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum FeedMetadata {
    InvalidAccount(Pubkey),
    SnapshotStart(Option<Pubkey>),
    SnapshotEnd(Option<Pubkey>),
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_snapshot_gta(
    rpc_http_url: &str,
    owner_id: &Pubkey,
) -> anyhow::Result<CustomSnapshotProgramAccounts> {
    let rpc_client = Arc::new(RpcClient::new_with_timeout_and_commitment(
        rpc_http_url.to_string(),
        Duration::from_secs(60 * 20),
        CommitmentConfig::confirmed(),
    ));
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(165),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                32,
                owner_id.to_bytes().as_slice(),
            )),
        ]),
        account_config: Default::default(),
        with_context: Some(true),
    };

    // println!("{}", serde_json::to_string(&config)?);

    let token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
    let result =
        get_compressed_program_account_rpc(&rpc_client, &HashSet::from([token_program]), config)
            .await?;

    Ok(CustomSnapshotProgramAccounts {
        slot: result.0,
        accounts: result.1,
        program_id: Some(token_program),
        missing_accounts: vec![],
    })
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_snapshot_gpa(
    rpc_http_url: &str,
    program_id: &Pubkey,
    use_compression: bool,
) -> anyhow::Result<CustomSnapshotProgramAccounts> {
    let result = if use_compression {
        get_compressed_program_account(rpc_http_url, &[*program_id].into_iter().collect()).await?
    } else {
        get_uncompressed_program_account(rpc_http_url, program_id.to_string()).await?
    };

    Ok(CustomSnapshotProgramAccounts {
        slot: result.0,
        accounts: result.1,
        program_id: Some(*program_id),
        missing_accounts: vec![],
    })
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_snapshot_gma(
    rpc_http_url: &str,
    keys: &[Pubkey],
) -> anyhow::Result<CustomSnapshotProgramAccounts> {
    let keys = keys.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    feeds_get_snapshot_gma(rpc_http_url, keys).await
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RpcKeyedCompressedAccount {
    pub p: String,
    pub a: String,
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_compressed_program_account(
    rpc_url: &str,
    filters: &HashSet<Pubkey>,
) -> anyhow::Result<(u64, Vec<AccountWrite>)> {
    // setting larget timeout because gPA can take a lot of time
    let rpc_client = Arc::new(RpcClient::new_with_timeout_and_commitment(
        rpc_url.to_string(),
        Duration::from_secs(60 * 20),
        CommitmentConfig::finalized(),
    ));
    let config = RpcProgramAccountsConfig {
        filters: None,
        with_context: Some(true),
        ..Default::default()
    };

    get_compressed_program_account_rpc(&rpc_client, filters, config).await
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_compressed_program_account_rpc(
    rpc_client: &RpcClient,
    filters: &HashSet<Pubkey>,
    config: RpcProgramAccountsConfig,
) -> anyhow::Result<(u64, Vec<AccountWrite>)> {
    let config = RpcProgramAccountsConfig {
        with_context: Some(true),
        ..config
    };

    let mut snap_result = vec![];
    let mut min_slot = u64::MAX;

    // use getGPA compressed if available
    for program_id in filters.iter() {
        info!("gPA for {}", program_id);

        let result = rpc_client
            .send::<OptionalContext<Vec<RpcKeyedCompressedAccount>>>(
                solana_client::rpc_request::RpcRequest::Custom {
                    method: "getProgramAccountsCompressed",
                },
                json!([program_id.to_string(), config]),
            )
            .await;

        // failed to get over compressed program accounts
        match result {
            Ok(OptionalContext::Context(response)) => {
                info!("Received compressed data for {}", program_id);
                let updated_slot = response.context.slot;
                min_slot = updated_slot.min(min_slot);

                for key_account in response.value {
                    let base64_decoded =
                        base64::engine::general_purpose::STANDARD.decode(&key_account.a)?;
                    // decompress all the account information
                    let uncompressed = lz4::block::decompress(&base64_decoded, None)?;
                    let shared_data = bincode::deserialize::<AccountSharedData>(&uncompressed)?;
                    let pubkey = Pubkey::from_str(&key_account.p).unwrap();
                    let account: Account = shared_data.into();
                    snap_result.push(account_write_from(
                        pubkey,
                        updated_slot,
                        SNAP_ACCOUNT_WRITE_VERSION,
                        account,
                    ));
                }

                info!(
                    "Decompressed snapshot for {} with {} accounts",
                    program_id,
                    snap_result.len()
                );
            }
            Err(e) => {
                anyhow::bail!(
                    "failed to get program {} account snapshot: {}",
                    program_id,
                    e
                )
            }
            _ => {
                anyhow::bail!(
                    "failed to get program {} account snapshot (unknown reason)",
                    program_id
                )
            }
        }
    }

    Ok((min_slot, snap_result))
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_uncompressed_program_account_rpc(
    rpc_client: &RpcClient,
    filters: &HashSet<Pubkey>,
    config: RpcProgramAccountsConfig,
) -> anyhow::Result<(u64, Vec<AccountWrite>)> {
    let slot = rpc_client.get_slot().await?;
    let config = RpcProgramAccountsConfig {
        with_context: Some(true),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            min_context_slot: None,
            commitment: config.account_config.commitment,
            data_slice: config.account_config.data_slice,
        },
        filters: config.filters,
    };

    let mut snap_result = vec![];
    let mut min_slot = u64::MAX;

    // use getGPA compressed if available
    for program_id in filters.iter() {
        info!("gPA for {}", program_id);
        min_slot = slot.min(min_slot);
        let account_snapshot = rpc_client
            .get_program_accounts_with_config(&program_id, config.clone())
            .await
            .map_err_anyhow()?;
        tracing::log::debug!("gpa snapshot received {}", program_id);

        let iter = account_snapshot.iter().map(|(pk, account)| {
            account_write_from(*pk, slot, SNAP_ACCOUNT_WRITE_VERSION, account.clone())
        });
        snap_result.extend(iter);
    }

    Ok((min_slot, snap_result))
}

// called on startup to get the required accounts, few calls with some 100 thousand accounts
#[tracing::instrument(skip_all, level = "trace")]
pub async fn get_uncompressed_program_account(
    rpc_url: &str,
    program_id: String,
) -> anyhow::Result<(u64, Vec<AccountWrite>)> {
    let result = feeds_get_snapshot_gpa(rpc_url, program_id).await?;

    Ok(result)
}

pub async fn feeds_get_snapshot_gpa(
    rpc_http_url: &str,
    program_id: String,
) -> anyhow::Result<(u64, Vec<AccountWrite>)> {
    let rpc_client = http::connect::<RpcAccountsScanClient>(rpc_http_url)
        .await
        .map_err_anyhow()?;

    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::finalized()),
        data_slice: None,
        min_context_slot: None,
    };
    let program_accounts_config = RpcProgramAccountsConfig {
        filters: None,
        with_context: Some(true),
        account_config: account_info_config.clone(),
    };

    tracing::log::debug!("requesting gpa snapshot {}", program_id);
    let account_snapshot = rpc_client
        .get_program_accounts(program_id.clone(), Some(program_accounts_config.clone()))
        .await
        .map_err_anyhow()?;
    tracing::log::debug!("gpa snapshot received {}", program_id);

    match account_snapshot {
        OptionalContext::Context(snapshot) => {
            let snapshot_slot = snapshot.context.slot;
            let mut accounts = vec![];

            for acc in snapshot.value {
                let (key, account) = (acc.pubkey, acc.account);
                let pubkey = Pubkey::from_str(key.as_str()).unwrap();
                let account: Account = account.decode().unwrap();
                accounts.push(account_write_from(
                    pubkey,
                    snapshot_slot,
                    SNAP_ACCOUNT_WRITE_VERSION,
                    account,
                ));
            }

            Ok((snapshot_slot, accounts))
        }
        OptionalContext::NoContext(_) => anyhow::bail!("bad snapshot format"),
    }
}

async fn feeds_get_snapshot_gma(
    rpc_http_url: &str,
    ids: Vec<String>,
) -> anyhow::Result<CustomSnapshotProgramAccounts> {
    let rpc_client = http::connect::<RpcAccountsScanClient>(rpc_http_url)
        .await
        .map_err_anyhow()?;

    let account_info_config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::finalized()),
        data_slice: None,
        min_context_slot: None,
    };

    tracing::log::debug!("requesting gma snapshot {:?}", ids);
    let account_snapshot_response = rpc_client
        .get_multiple_accounts(ids.clone(), Some(account_info_config))
        .await
        .map_err_anyhow()?;
    tracing::log::debug!("gma snapshot received {:?}", ids);

    let first_full_shot = account_snapshot_response.context.slot;

    let acc: Vec<(String, Option<UiAccount>)> = ids
        .iter()
        .zip(account_snapshot_response.value)
        .map(|x| (x.0.clone(), x.1))
        .collect();

    let mut accounts = vec![];
    let mut missing_accounts = vec![];

    for (key, account) in acc {
        let pubkey = Pubkey::from_str(key.as_str()).unwrap();
        if let Some(account) = account {
            let account: Account = account.decode().unwrap();
            accounts.push(account_write_from(
                pubkey,
                first_full_shot,
                SNAP_ACCOUNT_WRITE_VERSION,
                account,
            ));
        } else {
            missing_accounts.push(pubkey);
        }
    }

    Ok(CustomSnapshotProgramAccounts {
        slot: first_full_shot,
        accounts,
        missing_accounts,
        program_id: None,
    })
}

/// Fetch multiple account using one request per chunk of `max_chunk_size` accounts
///
/// WARNING: some accounts requested may be missing from the result
pub async fn fetch_multiple_accounts(
    rpc: &RpcClient,
    all_keys: &[Pubkey],
    max_chunk_size: usize,
) -> anyhow::Result<Vec<(Pubkey, Account)>> {
    let config = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        ..RpcAccountInfoConfig::default()
    };

    let mut raw_results = vec![];

    for keys in all_keys.chunks(max_chunk_size) {
        let account_info_config = config.clone();
        let keys: Vec<Pubkey> = keys.to_vec();
        let req_res = rpc
            .get_multiple_accounts_with_config(&keys, account_info_config)
            .await?;

        let r: Vec<(Pubkey, Option<Account>)> = keys.into_iter().zip(req_res.value).collect();
        raw_results.push(r);
    }

    let result = raw_results
        .into_iter()
        .flatten()
        .filter_map(|(pubkey, account_opt)| account_opt.map(|acc| (pubkey, acc)))
        .collect::<Vec<_>>();

    Ok(result)
}

trait AnyhowWrap {
    type Value;
    fn map_err_anyhow(self) -> anyhow::Result<Self::Value>;
}

impl<T, E: std::fmt::Debug> AnyhowWrap for Result<T, E> {
    type Value = T;
    fn map_err_anyhow(self) -> anyhow::Result<Self::Value> {
        self.map_err(|err| anyhow::anyhow!("{:?}", err))
    }
}
