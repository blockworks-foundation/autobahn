use crate::config::Config;
use async_channel::Receiver;
use services_mango_lib::postgres_configuration::PostgresConfiguration;
use services_mango_lib::postgres_connection;
use solana_program::pubkey::Pubkey;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tokio_postgres::Client;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct PersistableState {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_amount: u64,
    pub max_accounts: usize,
    pub input_amount_in_dollars: f64,
    pub router_quote_output_amount: u64,
    pub jupiter_quote_output_amount: u64,
    pub router_simulation_is_success: bool,
    pub jupiter_simulation_is_success: bool,
    pub router_accounts: usize,
    pub jupiter_accounts: usize,
    pub router_output_amount_in_dollars: f64,
    pub jupiter_output_amount_in_dollars: f64,
    pub router_route: String,
    pub jupiter_route: String,
    pub router_actual_output_amount: u64,
    pub jupiter_actual_output_amount: u64,
    pub router_error: String,
    pub jupiter_error: String,
}

pub(crate) async fn persist_tx_state(
    config: &Config,
    postgres_config: &PostgresConfiguration,
    receiver: Receiver<PersistableState>,
    exit_flag: Arc<AtomicBool>,
) {
    let mut last_error = Instant::now();
    let mut error_connecting: usize = 0;
    const ERROR_COUNT_SPAN: Duration = Duration::from_secs(10);
    const MAX_SEQUENTIAL_ERROR: usize = 10;
    while error_connecting < MAX_SEQUENTIAL_ERROR {
        let connection = if config.persist {
            match postgres_connection::connect(postgres_config).await {
                Ok(c) => Some(c),
                Err(e) => {
                    error!("failed to connect to SQL server {e:?}...");
                    if last_error.elapsed() < ERROR_COUNT_SPAN {
                        error_connecting += 1;
                        last_error = Instant::now();
                    }
                    continue;
                }
            }
        } else {
            None
        };

        let mut scheduled_exit = false;
        loop {
            if exit_flag.load(Ordering::Relaxed) {
                scheduled_exit = true;
                warn!("shutting down persister...");
                break;
            }

            let Ok(tx) = receiver.recv().await else {
                scheduled_exit = true;
                warn!("shutting down persister...");
                break;
            };

            info!(
                %tx.input_mint,
                %tx.output_mint,
                tx.input_amount,
                tx.input_amount_in_dollars,
                tx.max_accounts,
                tx.jupiter_quote_output_amount,
                tx.jupiter_simulation_is_success,
                tx.router_quote_output_amount,
                tx.router_simulation_is_success,
                tx.router_accounts,
                tx.jupiter_accounts,
                tx.router_output_amount_in_dollars,
                tx.jupiter_output_amount_in_dollars,
                tx.router_route,
                tx.jupiter_route,
                tx.router_actual_output_amount,
                tx.jupiter_actual_output_amount,
                tx.router_error,
                tx.jupiter_error,
                "State"
            );

            if config.persist {
                match persist(tx, &connection.as_ref().unwrap().0).await {
                    Ok(_) => {}
                    Err(e) => {
                        warn!("persist failed with error => {:?}", e);
                        break;
                    }
                }
            }
        }

        if !scheduled_exit && last_error.elapsed() < ERROR_COUNT_SPAN {
            error_connecting += 1;
            last_error = Instant::now();
        }
    }
}

async fn persist(state: PersistableState, client: &Client) -> anyhow::Result<()> {
    let input_amount = state.input_amount as i64;
    let input_amount_in_dollars = state.input_amount_in_dollars;
    let input_mint = state.input_mint.to_string();
    let output_mint = state.output_mint.to_string();
    let router_is_success = state.router_simulation_is_success;
    let jupiter_is_success = state.jupiter_simulation_is_success;
    let router_quote = state.router_quote_output_amount as i64;
    let jupiter_quote = state.jupiter_quote_output_amount as i64;
    let router_actual_output_amount = state.router_actual_output_amount as i64;
    let jupiter_actual_output_amount = state.jupiter_actual_output_amount as i64;
    let max_accounts = state.max_accounts as i64;
    let router_accounts = state.router_accounts as i64;
    let jupiter_accounts = state.jupiter_accounts as i64;
    let router_output_amount_in_dollars = state.router_output_amount_in_dollars;
    let jupiter_output_amount_in_dollars = state.jupiter_output_amount_in_dollars;
    let router_error = state.router_error;
    let jupiter_error = state.jupiter_error;
    let timestamp = chrono::Utc::now();

    let query = postgres_query::query!(
        "INSERT INTO router.comparison \
            (input_mint, output_mint, input_amount, input_amount_in_dollars, max_accounts, router_quote_output_amount, jupiter_quote_output_amount, router_simulation_success, jupiter_simulation_success, router_accounts, jupiter_accounts, router_output_amount_in_dollars, jupiter_output_amount_in_dollars, router_route, jupiter_route, router_actual_output_amount, jupiter_actual_output_amount, router_error, jupiter_error, timestamp) \
            VALUES ($input_mint, $output_mint, $input_amount, $input_amount_in_dollars, $max_accounts, $router_quote, $jupiter_quote, $router_is_success, $jupiter_is_success, $router_accounts, $jupiter_accounts, $router_output_amount_in_dollars, $jupiter_output_amount_in_dollars, $router_route, $jupiter_route, $router_actual_output_amount, $jupiter_actual_output_amount, $router_error, $jupiter_error, $timestamp)",
        input_mint,
        output_mint,
        input_amount,
        input_amount_in_dollars,
        max_accounts,
        router_quote,
        jupiter_quote,
        router_is_success,
        jupiter_is_success,
        router_accounts,
        jupiter_accounts,
        router_output_amount_in_dollars,
        jupiter_output_amount_in_dollars,
        router_route = state.router_route,
        jupiter_route = state.jupiter_route,
        router_actual_output_amount,
        jupiter_actual_output_amount,
        router_error,
        jupiter_error,
        timestamp,
    );

    query.execute(client).await?;
    Ok(())
}
