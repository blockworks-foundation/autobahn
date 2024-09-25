use crate::persister::PersistableTx;
use futures_util::StreamExt;
use router_feed_lib::grpc_tx_watcher::ExecTx;
use router_feed_lib::{grpc_tx_watcher, utils};
use std::fs::File;
use std::io::Read;
use std::sync::{atomic, Arc};
use tokio::sync::broadcast;
use tracing::{error, info};

mod config;
mod persister;
mod tx_watcher;

#[tokio::main(worker_threads = 10)]
async fn main() -> anyhow::Result<()> {
    utils::tracing_subscriber_init();
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

    let exit_flag: Arc<atomic::AtomicBool> = Arc::new(atomic::AtomicBool::new(false));
    let (exit_sender, _) = broadcast::channel(1);
    {
        let exit_flag = exit_flag.clone();
        let exit_sender = exit_sender.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            info!("Received SIGINT, shutting down...");
            exit_flag.store(true, atomic::Ordering::Relaxed);
            exit_sender.send(()).unwrap();
        });
    }

    let (tx_sender, tx_receiver) = async_channel::unbounded::<ExecTx>();
    let (persistable_sender, persistable_receiver) = async_channel::unbounded::<PersistableTx>();

    let ef = exit_sender.subscribe();
    let tx_sender_job = tokio::spawn(async move {
        grpc_tx_watcher::process_tx_events(&config.source, tx_sender, ef).await;
    });

    let ef = exit_sender.subscribe();
    let watcher_job = tokio::spawn(async move {
        tx_watcher::watch_tx_events(tx_receiver, persistable_sender, ef).await;
    });

    let ef = exit_flag.clone();
    let persister_job = tokio::spawn(async move {
        persister::persist_tx_state(&config.metrics, &config.postgres, persistable_receiver, ef)
            .await;
    });

    let mut jobs: futures::stream::FuturesUnordered<_> =
        vec![tx_sender_job, watcher_job, persister_job]
            .into_iter()
            .collect();

    jobs.next().await;
    error!("A critical job exited, aborting run..");

    Ok(())
}
