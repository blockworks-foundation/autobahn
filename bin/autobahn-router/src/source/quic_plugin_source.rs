use itertools::Itertools;
use jsonrpc_core::futures::StreamExt;

use quic_geyser_common::filters::MemcmpFilter;
use quic_geyser_common::types::connections_parameters::ConnectionParameters;
use solana_sdk::pubkey::Pubkey;

use anchor_spl::token::spl_token;
use async_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use std::{collections::HashMap, env, time::Duration};
use tracing::*;

use crate::metrics;
use mango_feeds_connector::{chain_data::SlotStatus, SlotUpdate};
use quic_geyser_common::message::Message;
use router_config_lib::{AccountDataSourceConfig, QuicSourceConfig};
use router_feed_lib::account_write::{AccountOrSnapshotUpdate, AccountWrite};
use router_feed_lib::get_program_account::{
    get_snapshot_gma, get_snapshot_gpa, get_snapshot_gta, CustomSnapshotProgramAccounts,
    FeedMetadata,
};
use solana_program::clock::Slot;
use tokio::sync::Semaphore;

// limit number of concurrent gMA/gPA requests
const MAX_PARALLEL_HEAVY_RPC_REQUESTS: usize = 4;

#[allow(clippy::large_enum_variant)]
pub enum SourceMessage {
    QuicMessage(Message),
    Snapshot(CustomSnapshotProgramAccounts),
}

pub async fn feed_data_geyser(
    quic_source_config: &QuicSourceConfig,
    snapshot_config: AccountDataSourceConfig,
    subscribed_accounts: &HashSet<Pubkey>,
    subscribed_programs: &HashSet<Pubkey>,
    subscribed_token_accounts: &HashSet<Pubkey>,
    sender: async_channel::Sender<SourceMessage>,
) -> anyhow::Result<()> {
    let use_compression = snapshot_config.rpc_support_compression.unwrap_or(false);
    let number_of_accounts_per_gma = snapshot_config.number_of_accounts_per_gma.unwrap_or(100);

    let snapshot_rpc_http_url = match &snapshot_config.rpc_http_url.chars().next().unwrap() {
        '$' => env::var(&snapshot_config.rpc_http_url[1..])
            .expect("reading connection string from env"),
        _ => snapshot_config.rpc_http_url.clone(),
    };
    info!("connecting to quic source {:?}", quic_source_config);

    let (quic_client, mut stream, _jh) = quic_geyser_client::non_blocking::client::Client::new(
        quic_source_config.connection_string.clone(),
        ConnectionParameters {
            enable_gso: quic_source_config.enable_gso.unwrap_or(true),
            ..Default::default()
        },
    )
    .await?;

    let mut subscriptions = vec![];

    let subscribed_program_filter = subscribed_programs.iter().map(|x| {
        quic_geyser_common::filters::Filter::Account(quic_geyser_common::filters::AccountFilter {
            owner: Some(*x),
            accounts: None,
            filters: None,
        })
    });
    subscriptions.extend(subscribed_program_filter);

    let subscribed_token_accounts_filter = subscribed_programs.iter().map(|x| {
        quic_geyser_common::filters::Filter::Account(quic_geyser_common::filters::AccountFilter {
            owner: Some(Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()),
            accounts: None,
            filters: Some(vec![
                quic_geyser_common::filters::AccountFilterType::Datasize(165),
                quic_geyser_common::filters::AccountFilterType::Memcmp(MemcmpFilter {
                    offset: 32,
                    data: quic_geyser_common::filters::MemcmpFilterData::Bytes(
                        x.to_bytes().to_vec(),
                    ),
                }),
            ]),
        })
    });
    subscriptions.extend(subscribed_token_accounts_filter);

    subscriptions.push(quic_geyser_common::filters::Filter::Account(
        quic_geyser_common::filters::AccountFilter {
            accounts: Some(subscribed_accounts.clone()),
            owner: None,
            filters: None,
        },
    ));

    subscriptions.push(quic_geyser_common::filters::Filter::Slot);
    quic_client.subscribe(subscriptions).await?;

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
    let mut max_finalized_slot = 0;

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

    let mut last_message_received_at = Instant::now();

    loop {
        tokio::select! {
            update = stream.recv() => {
                let Some(mut message) = update
                else {
                    anyhow::bail!("geyser plugin has closed the stream");
                };
                // use account and slot updates to trigger snapshot loading
                match &mut message {
                    Message::SlotMsg(slot_update) => {
                        trace!("received slot update for slot {}", slot_update.slot);
                        let commitment_config = slot_update.commitment_config;

                        debug!(
                            "slot_update: {} ({:?})",
                            slot_update.slot,
                            commitment_config
                        );

                        if commitment_config.is_finalized() {
                            if first_full_slot == u64::MAX {
                                // TODO: is this equivalent to before? what was highesy_write_slot?
                                first_full_slot = slot_update.slot + 1;
                            }
                            // TODO rename rooted to finalized
                            if slot_update.slot > max_finalized_slot {
                                max_finalized_slot = slot_update.slot;
                            }

                            let waiting_for_snapshot_slot = max_finalized_slot <= first_full_slot + rooted_to_finalized_slots;

                            if waiting_for_snapshot_slot {
                                debug!("waiting for snapshot slot: rooted={}, first_full={}, slot={}", max_finalized_slot, first_full_slot, slot_update.slot);
                            }

                            if snapshot_needed && !waiting_for_snapshot_slot {
                                snapshot_needed = false;

                                debug!("snapshot slot reached - setting up snapshot tasks");

                                let permits_parallel_rpc_requests = Arc::new(Semaphore::new(MAX_PARALLEL_HEAVY_RPC_REQUESTS));

                                info!("Requesting snapshot from gMA for {} filter accounts", subscribed_accounts.len());
                                for pubkey_chunk in subscribed_accounts.iter().chunks(number_of_accounts_per_gma).into_iter() {
                                    let rpc_http_url = snapshot_rpc_http_url.clone();
                                    let account_ids = pubkey_chunk.map(|x| *x).collect_vec();
                                    let sender = snapshot_gma_sender.clone();
                                    let permits = permits_parallel_rpc_requests.clone();
                                    tokio::spawn(async move {
                                        let _permit = permits.acquire().await.unwrap();
                                        let snapshot = get_snapshot_gma(&rpc_http_url, &account_ids).await;
                                        match sender.send(snapshot) {
                                            Ok(_) => {}
                                            Err(_) => {
                                                warn!("Could not send snapshot, quic has probably reconnected");
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
                                                warn!("Could not send snapshot, quic has probably reconnected");
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
                                                warn!("Could not send snapshot, quic has probably reconnected");
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    },
                    Message::AccountMsg(info) => {
                        let slot = info.slot_identifier.slot;
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
                        } else if max_finalized_slot > 0 && info.slot_identifier.slot < max_finalized_slot - max_out_of_order_slots {
                            anyhow::bail!("received write {} slots back from max rooted slot {}", max_finalized_slot - slot, max_finalized_slot);
                        }
                    },
                    _ => {
                        // ignore all other quic update types
                    }
                }

                let elapsed = last_message_received_at.elapsed().as_millis();
                metrics::QUIC_NO_MESSAGE_FOR_DURATION_MS.set(elapsed as i64);
                last_message_received_at = Instant::now();

                // send the incremental updates to the channel
                sender.send(SourceMessage::QuicMessage(message)).await.expect("send success");
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

    let Some(quic_sources) = config.quic_sources.clone() else {
        return;
    };

    // note: caller in main.rs ensures this
    assert_eq!(quic_sources.len(), 1, "only one quic source supported");
    for quic_source in quic_sources.clone() {
        let msg_sender = msg_sender.clone();
        let sub_accounts = subscription_accounts.clone();
        let sub_programs = subscription_programs.clone();
        let sub_token_accounts = subscription_token_accounts.clone();

        let cfg = config.clone();

        source_jobs.push(tokio::spawn(async move {
            let mut error_count = 0;
            let mut last_error = Instant::now();

            // Continuously reconnect on failure
            loop {
                let out = feed_data_geyser(
                    &quic_source,
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

                metrics::QUIC_SOURCE_CONNECTION_RETRIES
                    .with_label_values(&[&quic_source.name])
                    .inc();

                tokio::time::sleep(std::time::Duration::from_secs(
                    quic_source.retry_connection_sleep_secs,
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
                warn!("shutting down quic_plugin_source because subtask failed...");
                break;
            },
            _ = exit.recv() => {
                warn!("shutting down quic_plugin_source...");
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
                        warn!("failed to process quic event: {:?}", e);
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

    metrics::QUIC_DEDUP_QUEUE.set(msg_receiver.len() as i64);
    match msg {
        SourceMessage::QuicMessage(message) => {
            match message {
                Message::AccountMsg(account_message) => {
                    metrics::QUIC_ACCOUNT_WRITES.inc();
                    metrics::QUIC_ACCOUNT_WRITE_QUEUE.set(account_write_queue_sender.len() as i64);
                    let solana_account = account_message.solana_account();

                    // Skip writes that a different server has already sent
                    let pubkey_writes = latest_write
                        .entry(account_message.slot_identifier.slot)
                        .or_default();
                    if !filters.contains(&account_message.pubkey) {
                        return;
                    }

                    let writes = pubkey_writes.entry(account_message.pubkey).or_insert(0);
                    if account_message.write_version <= *writes {
                        return;
                    }
                    *writes = account_message.write_version;
                    latest_write.retain(|&k, _| {
                        k >= account_message.slot_identifier.slot - latest_write_retention
                    });

                    account_write_queue_sender
                        .send(AccountOrSnapshotUpdate::AccountUpdate(AccountWrite {
                            pubkey: account_message.pubkey,
                            slot: account_message.slot_identifier.slot,
                            write_version: account_message.write_version,
                            lamports: account_message.lamports,
                            owner: account_message.owner,
                            executable: account_message.executable,
                            rent_epoch: account_message.rent_epoch,
                            data: solana_account.data,
                        }))
                        .await
                        .expect("send success");
                }
                Message::SlotMsg(slot_message) => {
                    metrics::QUIC_SLOT_UPDATES.inc();
                    metrics::QUIC_SLOT_UPDATE_QUEUE.set(slot_queue_sender.len() as i64);

                    let status = if slot_message.commitment_config.is_processed() {
                        SlotStatus::Processed
                    } else if slot_message.commitment_config.is_confirmed() {
                        SlotStatus::Confirmed
                    } else {
                        SlotStatus::Rooted
                    };

                    let slot_update = SlotUpdate {
                        slot: slot_message.slot,
                        parent: Some(slot_message.parent),
                        status,
                    };

                    slot_queue_sender
                        .send(slot_update)
                        .await
                        .expect("send success");
                }
                _ => {
                    // ignore update
                }
            }
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
            debug!(
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
                metrics::QUIC_SNAPSHOT_ACCOUNT_WRITES.inc();
                metrics::QUIC_ACCOUNT_WRITE_QUEUE.set(account_write_queue_sender.len() as i64);

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
            debug!("processing snapshot done");
            if let Err(e) = metadata_sender(FeedMetadata::SnapshotEnd(update.program_id)) {
                warn!("failed to send feed matadata event: {}", e);
            }
        }
    }
}
