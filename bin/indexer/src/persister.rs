use crate::config::MetricsConfig;
use async_channel::Receiver;
use services_mango_lib::postgres_configuration::PostgresConfiguration;
use services_mango_lib::postgres_connection;
use solana_sdk::signature::Signature;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_postgres::Client;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct PersistableTx {
    pub sig: Signature,
    pub is_success: bool,
    pub router_version: u8,
}

pub(crate) async fn persist_tx_state(
    config: &MetricsConfig,
    postgres_config: &PostgresConfiguration,
    receiver: Receiver<PersistableTx>,
    exit_flag: Arc<AtomicBool>,
) {
    let connection = if config.enabled {
        let Ok(c) = postgres_connection::connect(postgres_config).await else {
            error!("failed to connect to SQL server...");
            return;
        };
        Some(c)
    } else {
        None
    };

    let mut inserted = HashMap::new();

    loop {
        if exit_flag.load(Ordering::Relaxed) {
            warn!("shutting down persist_tx_state...");
            break;
        }

        let Ok(tx) = receiver.recv().await else {
            warn!("shutting down persist_tx_state...");
            break;
        };

        info!(
            sig = tx.sig.to_string(),
            tx.is_success, tx.router_version, "TX"
        );

        if config.enabled {
            if inserted.insert(tx.sig, Instant::now()).is_some() {
                continue;
            }

            match persist(tx, &connection.as_ref().unwrap().0).await {
                Ok(_) => {}
                Err(e) => {
                    warn!("persist failed with error => {:?}", e);
                }
            }

            if inserted.len() > 1000 {
                inserted.retain(|_, x| x.elapsed() < Duration::from_secs(3600));
            }
        }
    }
}

async fn persist(tx: PersistableTx, client: &Client) -> anyhow::Result<()> {
    // TODO FAS - Batch insert, handle errors, etc...
    let signature = tx.sig.to_string();
    let is_success = tx.is_success;
    let router_version = tx.router_version as i32;
    let timestamp = chrono::Utc::now();

    let query = postgres_query::query!(
        "INSERT INTO router.tx_history \
            (signature, is_success, router_version, timestamp) \
            VALUES($signature, $is_success, $router_version, $timestamp)",
        signature,
        is_success,
        router_version,
        timestamp,
    );

    query.execute(client).await?;
    Ok(())
}
