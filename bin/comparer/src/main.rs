use crate::persister::PersistableState;
use futures_util::StreamExt;
use router_feed_lib::utils;
use std::fs::File;
use std::io::Read;
use std::sync::{atomic, Arc};
use tokio::sync::broadcast;
use tracing::{error, info};

mod bot;
mod config;
mod persister;

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

    let (persistable_sender, persistable_receiver) = async_channel::unbounded::<PersistableState>();

    let ef = exit_sender.subscribe();
    let cf = config.clone();

    let bot_job = tokio::spawn(async move {
        match bot::run(&cf, persistable_sender, ef).await {
            Ok(_) => {}
            Err(e) => {
                error!("Bot job failed with {:?}", e);
            }
        };
    });

    let ef = exit_flag.clone();
    let persister_job = tokio::spawn(async move {
        persister::persist_tx_state(&config, &config.postgres, persistable_receiver, ef).await;
    });

    let mut jobs: futures::stream::FuturesUnordered<_> =
        vec![bot_job, persister_job].into_iter().collect();

    jobs.next().await;
    error!("A critical job exited, aborting run..");

    Ok(())
}
