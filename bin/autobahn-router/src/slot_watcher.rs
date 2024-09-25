use router_config_lib::{string_or_env, AccountDataSourceConfig};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use std::time;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinHandle;

pub fn spawn_slot_watcher_job(config: &AccountDataSourceConfig) -> (JoinHandle<()>, Sender<u64>) {
    let (rpc_slot_sender, _) = broadcast::channel::<u64>(2048);
    let sender = rpc_slot_sender.clone();

    let processed_rpc = RpcClient::new_with_timeouts_and_commitment(
        string_or_env(config.rpc_http_url.clone()),
        time::Duration::from_secs(60), // request timeout
        CommitmentConfig::processed(),
        time::Duration::from_secs(60), // confirmation timeout
    );
    let slot_job = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;
        loop {
            interval.tick().await;
            let slot = processed_rpc.get_slot().await;
            if let Ok(slot) = slot {
                // ignore error for now
                let _err = sender.send(slot);
            }
        }
    });

    (slot_job, rpc_slot_sender)
}
