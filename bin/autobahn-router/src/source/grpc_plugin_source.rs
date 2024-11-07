use futures::stream::once;
use itertools::Itertools;
use jsonrpc_core::futures::StreamExt;

use solana_sdk::pubkey::Pubkey;

use tokio_stream::StreamMap;
use yellowstone_grpc_proto::tonic::{
    metadata::MetadataValue,
    transport::{Channel, ClientTlsConfig},
    Request,
};

use anchor_spl::token::spl_token;
use async_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use std::{collections::HashMap, env, time::Duration};
use tracing::*;

use yellowstone_grpc_proto::prelude::{
    geyser_client::GeyserClient, subscribe_update, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
};

use crate::metrics;
use mango_feeds_connector::{chain_data::SlotStatus, SlotUpdate};
use router_config_lib::{AccountDataSourceConfig, GrpcSourceConfig};
use router_feed_lib::account_write::{AccountOrSnapshotUpdate, AccountWrite};
use router_feed_lib::get_program_account::{
    get_snapshot_gma, get_snapshot_gpa, get_snapshot_gta, CustomSnapshotProgramAccounts,
    FeedMetadata,
};
use router_feed_lib::utils::make_tls_config;
use solana_program::clock::Slot;
use tokio::sync::Semaphore;
use yellowstone_grpc_proto::geyser::subscribe_request_filter_accounts_filter::Filter;
use yellowstone_grpc_proto::geyser::{
    subscribe_request_filter_accounts_filter_memcmp, SubscribeRequestFilterAccountsFilter,
    SubscribeRequestFilterAccountsFilterMemcmp, SubscribeUpdateAccountInfo, SubscribeUpdateSlot,
};
use yellowstone_grpc_proto::tonic::codec::CompressionEncoding;

const MAX_GRPC_ACCOUNT_SUBSCRIPTIONS: usize = 100;

// limit number of concurrent gMA/gPA requests
const MAX_PARALLEL_HEAVY_RPC_REQUESTS: usize = 4;

// GRPC network tuning
// see https://github.com/hyperium/tonic/blob/v0.10.2/tonic/src/transport/channel/mod.rs
const GPRC_CLIENT_BUFFER_SIZE: usize = 65536; // default: 1024
                                              // see https://github.com/hyperium/hyper/blob/v0.14.28/src/proto/h2/client.rs#L45
const GRPC_CONN_WINDOW: u32 = 5242880; // 5MB
const GRPC_STREAM_WINDOW: u32 = 4194304; // default: 2MB

#[allow(clippy::large_enum_variant)]
pub enum SourceMessage {
    GrpcAccountUpdate(Slot, SubscribeUpdateAccountInfo),
    GrpcSlotUpdate(SubscribeUpdateSlot),
    Snapshot(CustomSnapshotProgramAccounts),
}

pub async fn feed_data_geyser(
    grpc_config: &GrpcSourceConfig,
    tls_config: Option<ClientTlsConfig>,
    snapshot_config: AccountDataSourceConfig,
    subscribed_accounts: &HashSet<Pubkey>,
    subscribed_programs: &HashSet<Pubkey>,
    subscribed_token_accounts: &HashSet<Pubkey>,
    sender: async_channel::Sender<SourceMessage>,
) -> anyhow::Result<()> {

    println!("feed_data_geyser a:{subscribed_accounts:?} p:{subscribed_programs:?} t:{subscribed_token_accounts:?}");
    let use_compression = snapshot_config.rpc_support_compression.unwrap_or(false);
    let number_of_accounts_per_gma = snapshot_config.number_of_accounts_per_gma.unwrap_or(100);
    let grpc_connection_string = match &grpc_config.connection_string.chars().next().unwrap() {
        '$' => env::var(&grpc_config.connection_string[1..])
            .expect("reading connection string from env"),
        _ => grpc_config.connection_string.clone(),
    };
    let snapshot_rpc_http_url = match &snapshot_config.rpc_http_url.chars().next().unwrap() {
        '$' => env::var(&snapshot_config.rpc_http_url[1..])
            .expect("reading connection string from env"),
        _ => snapshot_config.rpc_http_url.clone(),
    };
    info!("connecting to grpc source {}", grpc_connection_string);
    let endpoint = Channel::from_shared(grpc_connection_string)?;
    // TODO add grpc compression option
    let channel = if let Some(tls) = tls_config {
        endpoint.tls_config(tls)?
    } else {
        endpoint
    }
    .tcp_nodelay(true)
    .http2_adaptive_window(true)
    .buffer_size(GPRC_CLIENT_BUFFER_SIZE)
    .initial_connection_window_size(GRPC_CONN_WINDOW)
    .initial_stream_window_size(GRPC_STREAM_WINDOW)
    .connect()
    .await?;
    let token: Option<MetadataValue<_>> = match &grpc_config.token {
        Some(token) => {
            if token.is_empty() {
                None
            } else {
                match token.chars().next().unwrap() {
                    '$' => Some(
                        env::var(&token[1..])
                            .expect("reading token from env")
                            .parse()?,
                    ),
                    _ => Some(token.clone().parse()?),
                }
            }
        }
        None => None,
    };
    let mut client = GeyserClient::with_interceptor(channel, move |mut req: Request<()>| {
        if let Some(token) = &token {
            req.metadata_mut().insert("x-token", token.clone());
        }
        Ok(req)
    })
    .accept_compressed(CompressionEncoding::Gzip);

    let mut accounts_filter: HashSet<Pubkey> = HashSet::new();
    let mut accounts = HashMap::new();
    let mut slots = HashMap::new();
    let blocks = HashMap::new();
    let transactions = HashMap::new();
    let blocks_meta = HashMap::new();

    for program_id in subscribed_programs {
        accounts.insert(
            format!("client_owner_{program_id}").to_owned(),
            SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec![program_id.to_string()],
                filters: vec![],
            },
        );
    }

    for owner_id in subscribed_token_accounts {
        accounts.insert(
            format!("client_token_{owner_id}").to_owned(),
            SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec!["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()],
                filters: vec![
                    SubscribeRequestFilterAccountsFilter {
                        filter: Some(Filter::Datasize(165)),
                    },
                    SubscribeRequestFilterAccountsFilter {
                        filter: Some(Filter::Memcmp(SubscribeRequestFilterAccountsFilterMemcmp {
                            offset: 32,
                            data: Some(
                                subscribe_request_filter_accounts_filter_memcmp::Data::Bytes(
                                    owner_id.to_bytes().into_iter().collect(),
                                ),
                            ),
                        })),
                    },
                ],
            },
        );
    }

    if subscribed_accounts.len() > 0 {
        accounts.insert(
            "client_accounts".to_owned(),
            SubscribeRequestFilterAccounts {
                account: subscribed_accounts.iter().map(Pubkey::to_string).collect(),
                owner: vec![],
                filters: vec![],
            },
        );
        accounts_filter.extend(subscribed_accounts);
    }

    slots.insert(
        "client_slots".to_owned(),
        SubscribeRequestFilterSlots {
            filter_by_commitment: None,
        },
    );

    // could use "merge_streams" see geyser-grpc-connector
    let mut subscriptions = StreamMap::new();

    {
        let request = SubscribeRequest {
            blocks,
            blocks_meta,
            commitment: None,
            slots,
            transactions,
            accounts_data_slice: vec![],
            ping: None,
            ..Default::default()
        };
        let response = client.subscribe(once(async move { request })).await?;
        subscriptions.insert(usize::MAX, response.into_inner());
    }

    // account subscriptions may have at most 100 at a time
    let account_chunks = accounts
        .into_iter()
        .chunks(MAX_GRPC_ACCOUNT_SUBSCRIPTIONS)
        .into_iter()
        .map(|chunk| chunk.collect::<HashMap<String, SubscribeRequestFilterAccounts>>())
        .collect_vec();
    for (i, accounts) in account_chunks.into_iter().enumerate() {
        let request = SubscribeRequest {
            accounts,
            commitment: Some(CommitmentLevel::Processed as i32),
            accounts_data_slice: vec![],
            ping: None,
            ..Default::default()
        };
        let response = client.subscribe(once(async move { request })).await?;
        subscriptions.insert(i, response.into_inner());
    }

    // We can't get a snapshot immediately since the finalized snapshot would be for a
    // slot in the past and we'd be missing intermediate updates.
    //
    // Delay the request until the first slot we received all writes for becomes rooted
    // to avoid that problem - partially. The rooted slot will still be larger than the
    // finalized slot, so add a number of slots as a buffer.
    //
    // If that buffer isn't sufficient, there'll be a retry.

    // The first slot that we will receive _all_ account writes for
    let mut first_full_slot: u64 = u64::MAX;

    // If a snapshot should be performed when ready.
    let mut snapshot_needed = true;

    // The highest "rooted" slot that has been seen.
    let mut max_rooted_slot = 0;

    // Data for slots will arrive out of order. This value defines how many
    // slots after a slot was marked "rooted" we assume it'll not receive
    // any more account write information.
    //
    // This is important for the write_version mapping (to know when slots can
    // be dropped).
    let max_out_of_order_slots = 40;

    // Number of slots that we expect "finalized" commitment to lag
    // behind "rooted". This matters for getProgramAccounts based snapshots,
    // which will have "finalized" commitment.
    let mut rooted_to_finalized_slots = 30;

    let (snapshot_gma_sender, mut snapshot_gma_receiver) = tokio::sync::mpsc::unbounded_channel();
    // TODO log buffer size

    // The plugin sends a ping every 5s or so
    let fatal_idle_timeout = Duration::from_secs(15);
    let mut re_snapshot_interval = tokio::time::interval(Duration::from_secs(
        snapshot_config
            .re_snapshot_interval_secs
            .unwrap_or(60 * 60 * 12),
    ));
    re_snapshot_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    re_snapshot_interval.tick().await;

    // Highest slot that an account write came in for.
    let mut newest_write_slot: u64 = 0;

    #[derive(Clone, Debug)]
    struct WriteVersion {
        // Write version seen on-chain
        global: u64,
        // FIXME clarify,rename
        // The per-pubkey per-slot write version
        per_slot_write_version: u32,
    }

    // map slot -> (pubkey -> WriteVersion)
    //
    // Since the write_version is a private indentifier per node it can't be used
    // to deduplicate events from multiple nodes. Here we rewrite it such that each
    // pubkey and each slot has a consecutive numbering of writes starting at 1.
    //
    // That number will be consistent for each node.
    let mut slot_pubkey_writes = HashMap::<u64, HashMap<[u8; 32], WriteVersion>>::new();

    let mut last_message_received_at = Instant::now();

    loop {
        tokio::select! {
            update = subscriptions.next() => {
                let Some(data) = update
                else {
                    anyhow::bail!("geyser plugin has closed the stream");
                };
                use subscribe_update::UpdateOneof;
                let update = data.1?;
                // use account and slot updates to trigger snapshot loading
                match &update.update_oneof {
                    Some(UpdateOneof::Slot(slot_update)) => {
                        trace!("received slot update for slot {}", slot_update.slot);
                        let status = slot_update.status;

                        debug!(
                            "slot_update: {} ({})",
                            slot_update.slot,
                            slot_update.status
                        );

                        if status == CommitmentLevel::Finalized as i32 {
                            if first_full_slot == u64::MAX {
                                // TODO: is this equivalent to before? what was highesy_write_slot?
                                first_full_slot = slot_update.slot + 1;
                            }
                            // TODO rename rooted to finalized
                            if slot_update.slot > max_rooted_slot {
                                max_rooted_slot = slot_update.slot;

                                // drop data for slots that are well beyond rooted
                                slot_pubkey_writes.retain(|&k, _| k >= max_rooted_slot - max_out_of_order_slots);
                            }

                            let waiting_for_snapshot_slot = max_rooted_slot <= first_full_slot + rooted_to_finalized_slots;

                            if waiting_for_snapshot_slot {
                                debug!("waiting for snapshot slot: rooted={}, first_full={}, slot={}", max_rooted_slot, first_full_slot, slot_update.slot);
                            }

                            if snapshot_needed && !waiting_for_snapshot_slot {
                                snapshot_needed = false;

                                debug!("snapshot slot reached - setting up snapshot tasks");

                                let permits_parallel_rpc_requests = Arc::new(Semaphore::new(MAX_PARALLEL_HEAVY_RPC_REQUESTS));

                                info!("Requesting snapshot from gMA for {} filter accounts", accounts_filter.len());
                                for pubkey_chunk in accounts_filter.iter().chunks(number_of_accounts_per_gma).into_iter() {
                                    let rpc_http_url = snapshot_rpc_http_url.clone();
                                    let account_ids = pubkey_chunk.cloned().collect_vec();
                                    let sender = snapshot_gma_sender.clone();
                                    let permits = permits_parallel_rpc_requests.clone();
                                    tokio::spawn(async move {
                                        let _permit = permits.acquire().await.unwrap();
                                        let snapshot = get_snapshot_gma(&rpc_http_url, &account_ids).await;
                                        match sender.send(snapshot) {
                                            Ok(_) => {}
                                            Err(_) => {
                                                warn!("Could not send snapshot, grpc has probably reconnected");
                                            }
                                        }
                                    });
                                }

                                info!("Requesting snapshot from gPA for {} program filter accounts", subscribed_programs.len());
                                for program_id in subscribed_programs {
                                    let rpc_http_url = snapshot_rpc_http_url.clone();
                                    let program_id = *program_id;
                                    let sender = snapshot_gma_sender.clone();
                                    let permits = permits_parallel_rpc_requests.clone();
                                    tokio::spawn(async move {
                                        let _permit = permits.acquire().await.unwrap();
                                        let snapshot = get_snapshot_gpa(&rpc_http_url, &program_id, use_compression).await;
                                        match sender.send(snapshot) {
                                            Ok(_) => {}
                                            Err(_) => {
                                                warn!("Could not send snapshot, grpc has probably reconnected");
                                            }
                                        }
                                    });
                                }

                                info!("Requesting snapshot from gTA for {} owners filter accounts", subscribed_token_accounts.len());
                                for owner_id in subscribed_token_accounts {
                                    let rpc_http_url = snapshot_rpc_http_url.clone();
                                    let owner_id = owner_id.clone();
                                    let sender = snapshot_gma_sender.clone();
                                    let permits = permits_parallel_rpc_requests.clone();
                                    tokio::spawn(async move {
                                        let _permit = permits.acquire().await.unwrap();
                                        let snapshot = get_snapshot_gta(&rpc_http_url, &owner_id).await;
                                        match sender.send(snapshot) {
                                            Ok(_) => {}
                                            Err(_) => {
                                                warn!("Could not send snapshot, grpc has probably reconnected");
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    },
                    Some(UpdateOneof::Account(info)) => {
                        let slot = info.slot;
                        trace!("received account update for slot {}", slot);
                        if slot < first_full_slot {
                            // Don't try to process data for slots where we may have missed writes:
                            // We could not map the write_version correctly for them.
                            continue;
                        }

                        if slot > newest_write_slot {
                            newest_write_slot = slot;
                            debug!(
                                "newest_write_slot: {}",
                                newest_write_slot
                            );
                        } else if max_rooted_slot > 0 && info.slot < max_rooted_slot - max_out_of_order_slots {
                            anyhow::bail!("received write {} slots back from max rooted slot {}", max_rooted_slot - slot, max_rooted_slot);
                        }

                        let pubkey_writes = slot_pubkey_writes.entry(slot).or_default();
                        let mut info = info.account.clone().unwrap();

                        let pubkey_bytes = Pubkey::try_from(info.pubkey).unwrap().to_bytes();
                        let write_version_mapping = pubkey_writes.entry(pubkey_bytes).or_insert(WriteVersion {
                            global: info.write_version,
                            per_slot_write_version: 1, // write version 0 is reserved for snapshots
                        });

                        // We assume we will receive write versions for each pubkey in sequence.
                        // If this is not the case, logic here does not work correctly because
                        // a later write could arrive first.
                        if info.write_version < write_version_mapping.global {
                            anyhow::bail!("unexpected write version: got {}, expected >= {}", info.write_version, write_version_mapping.global);
                        }

                        // Rewrite the update to use the local write version and bump it
                        info.write_version = write_version_mapping.per_slot_write_version as u64;
                        write_version_mapping.per_slot_write_version += 1;
                    },
                    Some(UpdateOneof::Ping(_)) => {
                        trace!("received grpc ping");
                    },
                    Some(_) => {
                        // ignore all other grpc update types
                    },
                    None => {
                        unreachable!();
                    }
                }

                let elapsed = last_message_received_at.elapsed().as_millis();
                metrics::GRPC_NO_MESSAGE_FOR_DURATION_MS.set(elapsed as i64);
                last_message_received_at = Instant::now();

                // send the incremental updates to the channel
                match update.update_oneof {
                    Some(UpdateOneof::Account(account_update)) => {
                        let info = account_update.account.unwrap();
                        sender.send(SourceMessage::GrpcAccountUpdate(account_update.slot as Slot, info)).await.expect("send success");
                    }
                    Some(UpdateOneof::Slot(slot_update)) => {
                        sender.send(SourceMessage::GrpcSlotUpdate(slot_update)).await.expect("send success");
                    }
                    _ => {}
                }
            },
            snapshot_message = snapshot_gma_receiver.recv() => {
                let Some(snapshot_result) = snapshot_message
                else {
                    anyhow::bail!("snapshot channel closed");
                };
                let snapshot = snapshot_result?;
                debug!("snapshot (program={}, m_accounts={}) is for slot {}, first full slot was {}",
                    snapshot.program_id.map(|x| x.to_string()).unwrap_or("none".to_string()),
                    snapshot.accounts.len(),
                    snapshot.slot,
                    first_full_slot);

                if snapshot.slot < first_full_slot {
                    warn!(
                        "snapshot is too old: has slot {}, expected {} minimum - request another one but also use this snapshot",
                        snapshot.slot,
                        first_full_slot
                    );
                    // try again in another 25 slots
                    snapshot_needed = true;
                    rooted_to_finalized_slots += 25;
                }

                // New - Don't care if the snapshot is old, we want startup to work anyway
                // If an edge is not working properly, it will be disabled when swapping it
                sender
                    .send(SourceMessage::Snapshot(snapshot))
                    .await
                    .expect("send success");

            },
            _ = tokio::time::sleep(fatal_idle_timeout) => {
                anyhow::bail!("geyser plugin hasn't sent a message in too long");
            }
            _ = re_snapshot_interval.tick() => {
                info!("Re-snapshot hack");
                snapshot_needed = true;
            }
        }
    }
}

pub async fn process_events(
    config: AccountDataSourceConfig,
    subscription_accounts: HashSet<Pubkey>,
    subscription_programs: HashSet<Pubkey>,
    subscription_token_accounts: HashSet<Pubkey>,
    filters: HashSet<Pubkey>,
    account_write_queue_sender: async_channel::Sender<AccountOrSnapshotUpdate>,
    metdata_write_queue_sender: Option<async_channel::Sender<FeedMetadata>>,
    slot_queue_sender: async_channel::Sender<SlotUpdate>,
    mut exit: tokio::sync::broadcast::Receiver<()>,
) {
    // Subscribe to geyser
    let (msg_sender, msg_receiver) =
        async_channel::bounded::<SourceMessage>(config.dedup_queue_size);
    let mut source_jobs = vec![];

    let Some(grpc_sources) = config.grpc_sources.clone() else {
        return;
    };

    // note: caller in main.rs ensures this
    assert_eq!(grpc_sources.len(), 1, "only one grpc source supported");
    for grpc_source in grpc_sources.clone() {
        let msg_sender = msg_sender.clone();
        let sub_accounts = subscription_accounts.clone();
        let sub_programs = subscription_programs.clone();
        let sub_token_accounts = subscription_token_accounts.clone();

        // Make TLS config if configured
        let tls_config = grpc_source.tls.as_ref().map(make_tls_config).or_else(|| {
            if grpc_source.connection_string.starts_with("https") {
                Some(ClientTlsConfig::new())
            } else {
                None
            }
        });

        let cfg = config.clone();

        source_jobs.push(tokio::spawn(async move {
            let mut error_count = 0;
            let mut last_error = Instant::now();

            // Continuously reconnect on failure
            loop {
                let out = feed_data_geyser(
                    &grpc_source,
                    tls_config.clone(),
                    cfg.clone(),
                    &sub_accounts,
                    &sub_programs,
                    &sub_token_accounts,
                    msg_sender.clone(),
                );
                if last_error.elapsed() > Duration::from_secs(60 * 10) {
                    error_count = 0;
                }
                else if error_count > 10 {
                    error!("error during communication with the geyser plugin - retried too many time, exiting..");
                    break;
                }

                match out.await {
                    // happy case!
                    Err(err) => {
                        warn!(
                            "error during communication with the geyser plugin - retrying: {:?}",
                            err
                        );
                        last_error = Instant::now();
                        error_count += 1;
                    }
                    // this should never happen
                    Ok(_) => {
                        error!("feed_data must return an error, not OK - continue");
                        last_error = Instant::now();
                        error_count += 1;
                    }
                }

                metrics::GRPC_SOURCE_CONNECTION_RETRIES
                    .with_label_values(&[&grpc_source.name])
                    .inc();

                tokio::time::sleep(std::time::Duration::from_secs(
                    grpc_source.retry_connection_sleep_secs,
                ))
                .await;
            }
        }));
    }

    // slot -> (pubkey -> write_version)
    //
    // To avoid unnecessarily sending requests to SQL, we track the latest write_version
    // for each (slot, pubkey). If an already-seen write_version comes in, it can be safely
    // discarded.
    let mut latest_write = HashMap::<Slot, HashMap<Pubkey, u64>>::new();

    // Number of slots to retain in latest_write
    let latest_write_retention = 50;

    let mut source_jobs: futures::stream::FuturesUnordered<_> = source_jobs.into_iter().collect();

    loop {
        tokio::select! {
            _ = source_jobs.next() => {
                warn!("shutting down grpc_plugin_source because subtask failed...");
                break;
            },
            _ = exit.recv() => {
                warn!("shutting down grpc_plugin_source...");
                break;
            }
            msg = msg_receiver.recv() => {
                match msg {
                    Ok(msg) => {
                        process_account_updated_from_sources(&account_write_queue_sender,
                            &slot_queue_sender,
                            &msg_receiver,
                            msg,
                            &mut latest_write,
                            latest_write_retention,
                            &metdata_write_queue_sender,
                            &filters,
                            ).await ;
                    }
                    Err(e) => {
                        warn!("failed to process grpc event: {:?}", e);
                        break;
                    }
                };
            },
        };
    }

    // close all channels to notify downstream CSPs of error
    account_write_queue_sender.close();
    metdata_write_queue_sender.map(|s| s.close());
    slot_queue_sender.close();
}

// consume channel with snapshot and update data
async fn process_account_updated_from_sources(
    account_write_queue_sender: &Sender<AccountOrSnapshotUpdate>,
    slot_queue_sender: &Sender<SlotUpdate>,
    msg_receiver: &Receiver<SourceMessage>,
    msg: SourceMessage,
    latest_write: &mut HashMap<Slot, HashMap<Pubkey, u64>>,
    // in slots
    latest_write_retention: u64,
    // metric_account_writes: &mut MetricU64,
    // metric_account_queue: &mut MetricU64,
    // metric_dedup_queue: &mut MetricU64,
    // metric_slot_queue: &mut MetricU64,
    // metric_slot_updates: &mut MetricU64,
    // metric_snapshots: &mut MetricU64,
    // metric_snapshot_account_writes: &mut MetricU64,
    metdata_write_queue_sender: &Option<Sender<FeedMetadata>>,
    filters: &HashSet<Pubkey>,
) {
    let metadata_sender = |msg| {
        if let Some(sender) = &metdata_write_queue_sender {
            sender.send_blocking(msg)
        } else {
            Ok(())
        }
    };

    metrics::GRPC_DEDUP_QUEUE.set(msg_receiver.len() as i64);
    match msg {
        SourceMessage::GrpcAccountUpdate(slot, update) => {
            assert!(update.pubkey.len() == 32);
            assert!(update.owner.len() == 32);

            metrics::GRPC_ACCOUNT_WRITES.inc();
            metrics::GRPC_ACCOUNT_WRITE_QUEUE.set(account_write_queue_sender.len() as i64);

            // Skip writes that a different server has already sent
            let pubkey_writes = latest_write.entry(slot).or_default();
            let pubkey = Pubkey::try_from(update.pubkey.clone()).unwrap();
            if !filters.contains(&pubkey) {
                return;
            }

            let writes = pubkey_writes.entry(pubkey).or_insert(0);
            if update.write_version <= *writes {
                return;
            }
            *writes = update.write_version;
            latest_write.retain(|&k, _| k >= slot - latest_write_retention);

            let owner = Pubkey::try_from(update.owner.clone()).unwrap();

            account_write_queue_sender
                .send(AccountOrSnapshotUpdate::AccountUpdate(AccountWrite {
                    pubkey,
                    slot,
                    write_version: update.write_version,
                    lamports: update.lamports,
                    owner,
                    executable: update.executable,
                    rent_epoch: update.rent_epoch,
                    data: update.data,
                }))
                .await
                .expect("send success");
        }
        SourceMessage::GrpcSlotUpdate(update) => {
            metrics::GRPC_SLOT_UPDATES.inc();
            metrics::GRPC_SLOT_UPDATE_QUEUE.set(slot_queue_sender.len() as i64);

            let status = CommitmentLevel::try_from(update.status).map(|v| match v {
                CommitmentLevel::Processed => SlotStatus::Processed,
                CommitmentLevel::Confirmed => SlotStatus::Confirmed,
                CommitmentLevel::Finalized => SlotStatus::Rooted,
            });
            if status.is_err() {
                error!("unexpected slot status: {}", update.status);
                return;
            }
            let slot_update = SlotUpdate {
                slot: update.slot,
                parent: update.parent,
                status: status.expect("qed"),
            };

            slot_queue_sender
                .send(slot_update)
                .await
                .expect("send success");
        }
        SourceMessage::Snapshot(update) => {
            let label = if let Some(prg) = update.program_id {
                if prg == spl_token::ID {
                    "gpa(tokens)"
                } else {
                    "gpa"
                }
            } else {
                "gma"
            };
            metrics::ACCOUNT_SNAPSHOTS
                .with_label_values(&[&label])
                .inc();
            info!(
                "processing snapshot for program_id {} -> size={} & missing size={}...",
                update
                    .program_id
                    .map(|x| x.to_string())
                    .unwrap_or("".to_string()),
                update.accounts.len(),
                update.missing_accounts.len()
            );
            if let Err(e) = metadata_sender(FeedMetadata::SnapshotStart(update.program_id)) {
                warn!("failed to send feed matadata event: {}", e);
            }

            let mut updated_accounts = vec![];
            for account in update.accounts {
                metrics::GRPC_SNAPSHOT_ACCOUNT_WRITES.inc();
                metrics::GRPC_ACCOUNT_WRITE_QUEUE.set(account_write_queue_sender.len() as i64);

                if !filters.contains(&account.pubkey) {
                    continue;
                }

                updated_accounts.push(account);
            }
            account_write_queue_sender
                .send(AccountOrSnapshotUpdate::SnapshotUpdate(updated_accounts))
                .await
                .expect("send success");

            for account in update.missing_accounts {
                if let Err(e) = metadata_sender(FeedMetadata::InvalidAccount(account)) {
                    warn!("failed to send feed matadata event: {}", e);
                }
            }
            info!("processing snapshot done");
            if let Err(e) = metadata_sender(FeedMetadata::SnapshotEnd(update.program_id)) {
                warn!("failed to send feed matadata event: {}", e);
            }
        }
    }
}
