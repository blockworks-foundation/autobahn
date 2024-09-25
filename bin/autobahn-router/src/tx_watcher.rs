use crate::edge::Edge;
use crate::edge_updater::Dex;
use async_channel::Receiver;
use router_config_lib::{AccountDataSourceConfig, RoutingConfig};
use router_feed_lib::grpc_tx_watcher;
use router_feed_lib::grpc_tx_watcher::ExecTx;
use solana_program::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub fn spawn_tx_watcher_jobs(
    routing_config: &RoutingConfig,
    source_config: &AccountDataSourceConfig,
    dexs: &[Dex],
    exit_sender: &tokio::sync::broadcast::Sender<()>,
    exit_flag: Arc<AtomicBool>,
) -> (JoinHandle<()>, JoinHandle<()>) {
    let (tx_sender, tx_receiver) = async_channel::unbounded::<ExecTx>();
    let _ef = exit_flag.clone();
    let source_config = source_config.clone();
    let routing_config = routing_config.clone();
    let dexs = dexs.iter().map(|x| x.clone()).collect();

    let exit_receiver = exit_sender.subscribe();
    let tx_sender_job = tokio::spawn(async move {
        grpc_tx_watcher::process_tx_events(&source_config, tx_sender, exit_receiver).await;
    });

    let exit_receiver = exit_sender.subscribe();
    let tx_watcher_job = tokio::spawn(async move {
        watch_tx_events(routing_config, tx_receiver, &dexs, exit_receiver).await;
    });

    (tx_sender_job, tx_watcher_job)
}

pub async fn watch_tx_events(
    config: RoutingConfig,
    tx_receiver: Receiver<ExecTx>,
    dexs: &Vec<Dex>,
    mut exit_receiver: tokio::sync::broadcast::Receiver<()>,
) {
    let cooldown_duration_multihop =
        Duration::from_secs(config.cooldown_duration_multihop_secs.unwrap_or(15));
    let cooldown_duration_singlehop =
        Duration::from_secs(config.cooldown_duration_singlehop_secs.unwrap_or(45));

    let edges_per_pk: HashMap<Pubkey, Vec<Arc<Edge>>> = dexs
        .iter()
        .map(|dex| dex.edges_per_pk.clone())
        .flatten()
        .collect();

    loop {
        tokio::select! {
            _ = exit_receiver.recv() => {
                warn!("shutting down watch_tx_events...");
                break;
            },
            msg = tx_receiver.recv() => {
                match msg {
                    Ok(tx) => {
                        handle_tx(tx, &edges_per_pk, &cooldown_duration_multihop, &cooldown_duration_singlehop).await;
                    }
                    Err(_) => {
                        warn!("shutting down watch_tx_events...");
                        break;
                    }
                };
            },
        }
    }
}

async fn handle_tx(
    tx: ExecTx,
    edges_per_pk: &HashMap<Pubkey, Vec<Arc<Edge>>>,
    cooldown_multi: &Duration,
    cooldown_single: &Duration,
) {
    // This is very dirty
    // 1/ use accounts to try to find edges
    // 2/ in a multi hop, we don't know which one is fucked up,
    //    so cooldown everything but for a lesser time that for a single hop

    let instruction_data = tx.data.as_slice();
    let (_, instruction_data) = autobahn_executor::utils::read_u64(instruction_data);
    let (number_of_ix, _instruction_data) = autobahn_executor::utils::read_u8(instruction_data);
    let cooldown_duration = if number_of_ix > 1 {
        cooldown_multi
    } else {
        cooldown_single
    };

    let mut impacted_edges = HashSet::new();
    for account in &tx.accounts {
        let Some(edges) = edges_per_pk.get(account) else {
            continue;
        };

        for edge in edges {
            if impacted_edges.insert(edge.desc()) {
                if tx.is_success {
                    let mut writer = edge.state.write().unwrap();
                    writer.reset_cooldown();
                    info!("resetting edge {}", edge.desc());
                } else {
                    let mut writer = edge.state.write().unwrap();
                    writer.add_cooldown(&cooldown_duration);
                    info!("cooling down edge {}", edge.desc());
                }
            }
        }
    }

    if impacted_edges.is_empty() {
        warn!("didn't find edge");
    }
}
