use crate::persister::PersistableTx;
use async_channel::{Receiver, Sender};
use router_feed_lib::grpc_tx_watcher::ExecTx;
use tracing::{info, warn};

pub async fn watch_tx_events(
    receiver: Receiver<ExecTx>,
    sender: Sender<PersistableTx>,
    mut exit_flag: tokio::sync::broadcast::Receiver<()>,
) {
    info!("Starting to watch TX");

    loop {
        tokio::select! {
            _ = exit_flag.recv() => {
                warn!("shutting down watch_tx_events...");
                break;
            },
            msg = receiver.recv() => {
                match msg {
                    Err(_e) => {
                        warn!("shutting down watch_tx_events...");
                        break;
                    },
                    Ok(msg) => {
                        handle_tx(&sender, msg).await;
                    }
                }
            }
        }
    }
}

async fn handle_tx(sender: &Sender<PersistableTx>, msg: ExecTx) {
    let ix_discriminator = msg.data[0] & 15;
    let router_version = msg.data[0] >> 4;
    let is_success = msg.is_success;

    if ix_discriminator != autobahn_executor::Instructions::ExecuteSwapV3 as u8
        && ix_discriminator != autobahn_executor::Instructions::ExecuteSwapV2 as u8
    {
        return;
    }

    info!(router_version, is_success, "Swap TX Received");

    let is_insufficient_funds = msg.logs.iter().find(|x| x.contains("insufficient funds"));
    if is_insufficient_funds.is_some() {
        info!("ignoring tx {} - insufficient funds", msg.signature);
        return;
    }

    sender
        .send(crate::persister::PersistableTx {
            sig: msg.signature,
            is_success,
            router_version,
        })
        .await
        .expect("sending must succeed");
}
