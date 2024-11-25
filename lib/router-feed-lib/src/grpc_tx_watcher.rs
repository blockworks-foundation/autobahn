use futures::stream::once;
use itertools::Itertools;
use jsonrpc_core::futures::StreamExt;

use solana_sdk::pubkey::Pubkey;

use futures::pin_mut;
use yellowstone_grpc_proto::tonic::{
    metadata::MetadataValue,
    transport::{Channel, ClientTlsConfig},
    Request,
};

use async_channel::Sender;
use solana_sdk::signature::Signature;
use std::time::Instant;
use std::{collections::HashMap, env, time::Duration};
use tracing::*;

use yellowstone_grpc_proto::prelude::{
    geyser_client::GeyserClient, subscribe_update, SubscribeRequest, SubscribeUpdateTransaction,
};

use crate::utils::make_tls_config;
use router_config_lib::{AccountDataSourceConfig, GrpcSourceConfig};
use yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeRequestFilterTransactions};

#[derive(Debug, Clone)]
pub struct ExecTx {
    pub is_success: bool,
    pub data: Vec<u8>,
    pub accounts: Vec<Pubkey>,
    pub logs: Vec<String>,
    pub signature: Signature,
}

async fn feed_tx_geyser(
    grpc_config: &GrpcSourceConfig,
    tls_config: Option<ClientTlsConfig>,
    sender: async_channel::Sender<ExecTx>,
) -> anyhow::Result<()> {
    let grpc_connection_string = match &grpc_config.connection_string.chars().next().unwrap() {
        '$' => env::var(&grpc_config.connection_string[1..])
            .expect("reading connection string from env"),
        _ => grpc_config.connection_string.clone(),
    };

    info!("connecting to grpc source {}", grpc_connection_string);
    let endpoint = Channel::from_shared(grpc_connection_string)?;
    let channel = if let Some(tls) = tls_config {
        endpoint.tls_config(tls)?
    } else {
        endpoint
    }
    .connect()
    .await?;
    let token: Option<MetadataValue<_>> = match &grpc_config.token {
        Some(token) => match token.chars().next().unwrap() {
            '$' => Some(
                env::var(&token[1..])
                    .expect("reading token from env")
                    .parse()?,
            ),
            _ => Some(token.clone().parse()?),
        },
        None => None,
    };
    let mut client = GeyserClient::with_interceptor(channel, move |mut req: Request<()>| {
        if let Some(token) = &token {
            req.metadata_mut().insert("x-token", token.clone());
        }
        Ok(req)
    });

    let mut transactions = HashMap::new();
    transactions.insert(
        "execm".to_string(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: None,
            signature: None,
            account_include: vec![autobahn_executor::id().to_string()],
            account_exclude: vec![],
            account_required: vec![],
        },
    );

    let request = SubscribeRequest {
        commitment: Some(CommitmentLevel::Processed as i32),
        transactions,
        accounts_data_slice: vec![],
        ping: None,
        ..Default::default()
    };

    // The plugin sends a ping every 5s or so
    let fatal_idle_timeout = Duration::from_secs(60);

    let stream = client
        .subscribe(once(async move { request }))
        .await?
        .into_inner();
    pin_mut!(stream);

    loop {
        tokio::select! {
            update = stream.next() => {
                let Some(data) = update
                else {
                    anyhow::bail!("geyser plugin has closed the stream");
                };
                use subscribe_update::UpdateOneof;
                let update = data?;

                if let Some(UpdateOneof::Transaction(tx)) = update.update_oneof.as_ref() {
                    handle_tx(tx, &sender).await;
                }

            },
            _ = tokio::time::sleep(fatal_idle_timeout) => {
                anyhow::bail!("geyser plugin hasn't sent a message in too long");
            }
        }
    }
}

pub async fn handle_tx(tx: &SubscribeUpdateTransaction, sender: &async_channel::Sender<ExecTx>) {
    let Some(txu) = &tx.transaction else {
        return;
    };
    let Some(tx) = &txu.transaction else {
        return;
    };
    let Some(msg) = &tx.message else {
        return;
    };
    let Some(meta) = &txu.meta else {
        return;
    };

    for ix in &msg.instructions {
        let program_id: &Vec<u8> = &msg.account_keys[ix.program_id_index as usize];
        let Ok(program_id) = Pubkey::try_from(program_id.clone()) else {
            continue;
        };

        if program_id != autobahn_executor::id() {
            continue;
        }

        let accounts = msg
            .account_keys
            .iter()
            .chain(meta.loaded_writable_addresses.iter())
            .chain(meta.loaded_readonly_addresses.iter())
            .collect_vec();

        let logs = meta.log_messages.clone();

        sender
            .send(ExecTx {
                is_success: txu.meta.as_ref().map(|x| x.err.is_none()).unwrap_or(false),
                data: ix.data.clone(),
                accounts: ix
                    .accounts
                    .iter()
                    .map(|x| {
                        Pubkey::try_from(accounts[*x as usize].clone()).expect("invalid account")
                    })
                    .collect(),
                logs,
                signature: Signature::try_from(tx.signatures[0].clone()).expect("invalid sign"),
            })
            .await
            .expect("send success");
    }
}

pub async fn process_tx_events(
    config: &AccountDataSourceConfig,
    sender: async_channel::Sender<ExecTx>,
    mut exit: tokio::sync::broadcast::Receiver<()>,
) {
    // Subscribe to geyser
    let (msg_sender, msg_receiver) = async_channel::bounded::<ExecTx>(config.dedup_queue_size);
    let mut source_jobs = vec![];

    let Some(grpc_sources) = config.grpc_sources.clone() else {
        panic!("There should be atleast one grpc source specified for grpc tx watcher");
    };

    for grpc_source in grpc_sources.clone() {
        let msg_sender = msg_sender.clone();

        // Make TLS config if configured
        let tls_config = grpc_source.tls.as_ref().map(make_tls_config).or_else(|| {
            if grpc_source.connection_string.starts_with("https") {
                Some(ClientTlsConfig::new())
            } else {
                None
            }
        });

        source_jobs.push(tokio::spawn(async move {
            let mut error_count = 0;
            let mut last_error = Instant::now();

            // Continuously reconnect on failure
            loop {
                let out = feed_tx_geyser(&grpc_source, tls_config.clone(), msg_sender.clone());
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
                        error_count+=1;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(
                    grpc_source.retry_connection_sleep_secs,
                ))
                .await;
            }
        }));
    }

    let mut source_jobs: futures::stream::FuturesUnordered<_> = source_jobs.into_iter().collect();

    loop {
        tokio::select! {
            msg = msg_receiver.recv() => {
                match msg {
                    Ok(msg) => {
                        decode_and_forward_tx(&sender, msg).await;
                    }
                    Err(e) => {
                        warn!("failed to decode_and_forward_tx: {:?}", e);
                        break;
                    }
                };
            },
            _ = source_jobs.next() => {
                warn!("shutting down grpc_tx_watcher because subtask failed...");
                break;
            },
            _ = exit.recv() => {
                warn!("shutting down grpc_tx_watcher...");
                break;
            }
        };
    }
}

async fn decode_and_forward_tx(sender: &Sender<ExecTx>, msg: ExecTx) {
    let ix_discriminator = msg.data[0] & 15;
    if ix_discriminator != autobahn_executor::Instructions::ExecuteSwapV3 as u8
        && ix_discriminator != autobahn_executor::Instructions::ExecuteSwapV2 as u8
    {
        return;
    }

    let is_insufficient_funds = msg.logs.iter().find(|x| x.contains("insufficient funds"));
    if is_insufficient_funds.is_some() {
        debug!("ignoring tx - insufficient funds");
        return;
    }

    let is_invalid_ata = msg
        .logs
        .iter()
        .find(|x| x.contains("The program expected this account to be already initialized"));
    if is_invalid_ata.is_some() {
        debug!("ignoring tx - invalid ATA");
        return;
    }

    sender
        .send(msg.clone())
        .await
        .expect("sending must succeed");
}
