use crate::prelude::*;
use crate::server::errors::*;
use crate::server::route_provider::RouteProvider;
use axum::extract::Query;
use axum::response::Html;
use axum::{extract::Form, http::header::HeaderMap, routing, Json, Router};
use router_lib::model::quote_request::QuoteRequest;
use router_lib::model::quote_response::{QuoteAccount, QuoteResponse};
use router_lib::model::swap_request::{SwapForm, SwapRequest};
use router_lib::model::swap_response::{InstructionResponse, SwapIxResponse, SwapResponse};
use serde_json::Value;
use solana_program::address_lookup_table::AddressLookupTableAccount;
use solana_program::message::VersionedMessage;
use solana_sdk::account::ReadableAccount;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::signature::NullSigner;
use solana_sdk::transaction::VersionedTransaction;
use std::time::Instant;
use tokio::task::JoinHandle;
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};

use crate::alt::alt_optimizer;
use crate::ix_builder::SwapInstructionsBuilder;
use crate::liquidity::{LiquidityProvider, LiquidityProviderArcRw};
use crate::routing_types::Route;
use crate::server::alt_provider::AltProvider;
use crate::server::hash_provider::HashProvider;
use crate::{debug_tools, metrics};
use router_config_lib::Config;
use router_lib::dex::{AccountProvider, AccountProviderView, SwapMode};
use router_lib::model::liquidity_request::LiquidityRequest;
use router_lib::model::liquidity_response::LiquidityResponse;
use router_lib::model::quote_response::{RoutePlan, SwapInfo};

// make sure the transaction can be executed
const MAX_ACCOUNTS_PER_TX: usize = 64;
const MAX_TX_SIZE: usize = 1232;
const DEFAULT_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS: u64 = 10_000;

pub struct HttpServer {
    pub join_handle: JoinHandle<()>,
}

impl HttpServer {
    pub async fn start<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        route_provider: Arc<TRouteProvider>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        liquidity_provider: LiquidityProviderArcRw,
        ix_builder: Arc<TIxBuilder>,
        config: Config,
        exit: tokio::sync::broadcast::Receiver<()>,
    ) -> anyhow::Result<HttpServer> {
        let join_handle = HttpServer::new_server(
            route_provider,
            hash_provider,
            alt_provider,
            live_account_provider,
            liquidity_provider,
            ix_builder,
            config,
            exit,
        )
        .await?;

        Ok(HttpServer { join_handle })
    }
}

impl HttpServer {
    async fn new_server<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        route_provider: Arc<TRouteProvider>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        liquidity_provider: LiquidityProviderArcRw,
        ix_builder: Arc<TIxBuilder>,
        config: Config,
        exit: tokio::sync::broadcast::Receiver<()>,
    ) -> anyhow::Result<JoinHandle<()>> {
        let addr = &config.server.address;
        let alt = config.routing.lookup_tables.clone();
        let should_reprice = config
            .debug_config
            .as_ref()
            .map(|x| x.reprice_using_live_rpc)
            .unwrap_or(false);
        let reprice_frequency = if should_reprice {
            config
                .debug_config
                .as_ref()
                .map(|x| x.reprice_probability)
                .unwrap_or(1.0)
        } else {
            0.0
        };

        let app = Self::setup_router(
            alt,
            route_provider,
            hash_provider,
            alt_provider,
            live_account_provider,
            liquidity_provider,
            ix_builder,
            reprice_frequency,
        )?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let handle = axum::serve(listener, app).with_graceful_shutdown(Self::shutdown_signal(exit));

        info!("HTTP Server started at {}", addr);

        let join_handle = tokio::spawn(async move {
            handle.await.expect("HTTP Server failed");
        });

        Ok(join_handle)
    }

    async fn shutdown_signal(mut exit: tokio::sync::broadcast::Receiver<()>) {
        exit.recv()
            .await
            .expect("listening to exit broadcast failed");
        warn!("shutting down http server...");
    }

    async fn quote_handler<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        address_lookup_table_addresses: Vec<String>,
        route_provider: Arc<TRouteProvider>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        ix_builder: Arc<TIxBuilder>,
        reprice_probability: f64,
        Form(input): Form<QuoteRequest>,
    ) -> Result<Json<Value>, AppError> {
        let started_at = Instant::now();
        let input_mint = Pubkey::from_str(&input.input_mint)?;
        let output_mint = Pubkey::from_str(&input.output_mint)?;
        let swap_mode = input.swap_mode.or(input.mode).unwrap_or_default();
        let mut max_accounts = input.max_accounts.unwrap_or(64) as usize;

        let route = loop {
            let route_candidate = route_provider.best_quote(
                input_mint,
                output_mint,
                input.amount,
                max_accounts,
                swap_mode,
            )?;

            let (bytes, accounts_count) = Self::build_swap_tx(
                address_lookup_table_addresses.clone(),
                hash_provider.clone(),
                alt_provider.clone(),
                live_account_provider.clone(),
                ix_builder.clone(),
                &route_candidate,
                Pubkey::new_unique().to_string(),
                true,
                true,
                0,
                "0".to_string(),
                swap_mode,
                DEFAULT_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS,
            )
            .await?;

            let tx_size = bytes.len();
            if accounts_count <= MAX_ACCOUNTS_PER_TX && tx_size < MAX_TX_SIZE {
                break Ok(route_candidate);
            } else if max_accounts >= 10 {
                warn!("TX too big ({tx_size} bytes, {accounts_count} accounts), retrying with fewer accounts; max_accounts was {max_accounts}..");
                max_accounts -= 5;
            } else {
                break Err(anyhow::format_err!(
                    "TX too big ({tx_size} bytes, {accounts_count} accounts), aborting"
                ));
            }
        };

        let route: Route = route?;

        Self::log_repriced_amount(live_account_provider.clone(), reprice_probability, &route);

        let other_amount_threshold = if swap_mode == SwapMode::ExactOut {
            (route.in_amount as f64 * (10_000f64 + input.slippage_bps as f64) / 10_000f64).floor()
                as u64
        } else {
            ((route.out_amount as f64 * (10_000f64 - input.slippage_bps as f64)) / 10_000f64)
                .floor() as u64
        };

        let route_plan = route
            .steps
            .iter()
            .map(|step| RoutePlan {
                percent: 100,
                swap_info: Some(SwapInfo {
                    amm_key: step.edge.key().to_string(),
                    label: Some(step.edge.dex.name().to_string()),
                    input_mint: step.edge.input_mint.to_string(),
                    output_mint: step.edge.output_mint.to_string(),
                    in_amount: step.in_amount.to_string(),
                    out_amount: step.out_amount.to_string(),
                    fee_amount: step.fee_amount.to_string(),
                    fee_mint: step.fee_mint.to_string(),
                }),
            })
            .collect_vec();

        let accounts = match route.accounts {
            None => None,
            Some(a) => Some(
                a.iter()
                    .map(|x| QuoteAccount {
                        address: x.0.to_string(),
                        slot: x.1.slot,
                        data: x.1.account.data().iter().copied().collect::<Vec<u8>>(),
                    })
                    .collect(),
            ),
        };

        let context_slot = route.slot;
        let json_response = serde_json::json!(QuoteResponse {
            input_mint: input_mint.to_string(),
            in_amount: Some(route.in_amount.to_string()),
            output_mint: output_mint.to_string(),
            out_amount: route.out_amount.to_string(),
            other_amount_threshold: other_amount_threshold.to_string(),
            swap_mode: swap_mode.to_string(),
            slippage_bps: input.slippage_bps as i32,
            platform_fee: None, // TODO
            price_impact_pct: (route.price_impact_bps as f64 / 100.0).to_string(),
            route_plan,
            accounts,
            context_slot,
            time_taken: started_at.elapsed().as_secs_f64(),
        });

        Ok(Json(json_response))
    }

    async fn swap_handler<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        address_lookup_table_addresses: Vec<String>,
        route_provider: Arc<TRouteProvider>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        ix_builder: Arc<TIxBuilder>,
        reprice_probability: f64,
        Query(_query): Query<SwapForm>,
        Json(input): Json<SwapRequest>,
    ) -> Result<Json<Value>, AppError> {
        let route = route_provider.try_from(&input.quote_response)?;

        Self::log_repriced_amount(live_account_provider, reprice_probability, &route);

        let swap_mode: SwapMode = SwapMode::from_str(&input.quote_response.swap_mode)
            .map_err(|_| anyhow::Error::msg("Invalid SwapMode"))?;

        let compute_unit_price_micro_lamports = match input.compute_unit_price_micro_lamports {
            Some(price) => price,
            None => DEFAULT_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS,
        };

        let (bytes, _) = Self::build_swap_tx(
            address_lookup_table_addresses,
            hash_provider,
            alt_provider,
            ix_builder,
            &route,
            input.user_public_key,
            input.wrap_and_unwrap_sol,
            input.auto_create_out_ata,
            input.quote_response.slippage_bps,
            input.quote_response.other_amount_threshold,
            swap_mode,
            compute_unit_price_micro_lamports,
        )
        .await?;

        let json_response = serde_json::json!(SwapResponse {
            swap_transaction: bytes,
            last_valid_block_height: input.quote_response.context_slot,
            priorization_fee_lamports: compute_unit_price_micro_lamports / 1_000_000, // convert microlamports to lamports
        });

        Ok(Json(json_response))
    }

    fn log_repriced_amount<TAccountProvider: AccountProvider + Send + Sync + 'static>(
        live_account_provider: Arc<TAccountProvider>,
        reprice_probability: f64,
        route: &Route,
    ) {
        let should_reprice = rand::random::<f64>() < reprice_probability;
        if !should_reprice {
            return;
        }

        let repriced_out_amount = reprice(&route, live_account_provider);
        match repriced_out_amount {
            Ok(repriced_out) => {
                let diff = ((repriced_out as f64 / route.out_amount as f64) - 1.0) * 10000.0;
                let pair = format!(
                    "{}-{}",
                    debug_tools::name(&route.input_mint),
                    debug_tools::name(&route.output_mint)
                );
                metrics::REPRICING_DIFF_BPS
                    .with_label_values(&[&pair])
                    .set(diff);

                info!(
                    "Router quote: {}, Rpc quote: {}, Diff: {:.1}bps",
                    route.out_amount, repriced_out, diff
                );
            }
            Err(e) => {
                warn!("Repricing failed: {:?}", e)
            }
        }
    }

    async fn build_swap_tx<
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
    >(
        address_lookup_table_addresses: Vec<String>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        ix_builder: Arc<TIxBuilder>,
        route_plan: &Route,
        wallet_pk: String,
        wrap_unwrap_sol: bool,
        auto_create_out_ata: bool,
        slippage_bps: i32,
        other_amount_threshold: String,
        swap_mode: SwapMode,
        compute_unit_price_micro_lamports: u64,
    ) -> Result<(Vec<u8>, usize), AppError> {
        let wallet_pk = Pubkey::from_str(&wallet_pk)?;

        let ixs = ix_builder.build_ixs(
            live_account_provider,
            &wallet_pk,
            route_plan,
            wrap_unwrap_sol,
            auto_create_out_ata,
            slippage_bps,
            other_amount_threshold.parse()?,
            swap_mode,
        )?;

        let compute_budget_ixs = vec![
            ComputeBudgetInstruction::set_compute_unit_price(compute_unit_price_micro_lamports),
            ComputeBudgetInstruction::set_compute_unit_limit(ixs.cu_estimate),
        ];

        let transaction_addresses = ixs.accounts().into_iter().collect();
        let instructions = compute_budget_ixs
            .into_iter()
            .chain(ixs.setup_instructions.into_iter())
            .chain(vec![ixs.swap_instruction].into_iter())
            .chain(ixs.cleanup_instructions.into_iter())
            .collect_vec();

        let all_alts = Self::load_all_alts(address_lookup_table_addresses, alt_provider).await;
        let alts = alt_optimizer::get_best_alt(&all_alts, &transaction_addresses)?;
        let accounts = transaction_addresses.iter().unique().count()
            + alts.iter().map(|x| x.key).unique().count();

        let v0_message = solana_sdk::message::v0::Message::try_compile(
            &wallet_pk,
            instructions.as_slice(),
            alts.as_slice(),
            hash_provider.get_latest_hash().await?,
        )?;

        let message = VersionedMessage::V0(v0_message);
        let tx = VersionedTransaction::try_new(message, &[&NullSigner::new(&wallet_pk)])?;
        let bytes = bincode::serialize(&tx)?;

        Ok((bytes, accounts))
    }

    async fn swap_ix_handler<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        address_lookup_table_addresses: Vec<String>,
        route_provider: Arc<TRouteProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        ix_builder: Arc<TIxBuilder>,
        Query(_query): Query<SwapForm>,
        Json(input): Json<SwapRequest>,
    ) -> Result<Json<Value>, AppError> {
        let wallet_pk = Pubkey::from_str(&input.user_public_key)?;

        let route_plan = route_provider.try_from(&input.quote_response)?;
        let swap_mode: SwapMode = SwapMode::from_str(&input.quote_response.swap_mode)
            .map_err(|_| anyhow::Error::msg("Invalid SwapMode"))?;

        let compute_unit_price_micro_lamports = match input.compute_unit_price_micro_lamports {
            Some(price) => price,
            None => DEFAULT_COMPUTE_UNIT_PRICE_MICRO_LAMPORTS,
        };

        let ixs = ix_builder.build_ixs(
            live_account_provider,
            &wallet_pk,
            &route_plan,
            input.wrap_and_unwrap_sol,
            input.auto_create_out_ata,
            input.quote_response.slippage_bps,
            input.quote_response.other_amount_threshold.parse()?,
            swap_mode,
        )?;

        let transaction_addresses = ixs.accounts().into_iter().collect();
        let all_alts = Self::load_all_alts(address_lookup_table_addresses, alt_provider).await;
        let alts = alt_optimizer::get_best_alt(&all_alts, &transaction_addresses)?;

        let swap_ix = InstructionResponse::from_ix(ixs.swap_instruction)?;
        let setup_ixs: anyhow::Result<Vec<_>> = ixs
            .setup_instructions
            .into_iter()
            .map(|x| InstructionResponse::from_ix(x))
            .collect();
        let cleanup_ixs: anyhow::Result<Vec<_>> = ixs
            .cleanup_instructions
            .into_iter()
            .map(|x| InstructionResponse::from_ix(x))
            .collect();

        let compute_budget_ixs = vec![
            InstructionResponse::from_ix(ComputeBudgetInstruction::set_compute_unit_price(
                compute_unit_price_micro_lamports,
            ))?,
            InstructionResponse::from_ix(ComputeBudgetInstruction::set_compute_unit_limit(
                ixs.cu_estimate,
            ))?,
        ];

        let json_response = serde_json::json!(SwapIxResponse {
            token_ledger_instruction: None,
            compute_budget_instructions: Some(compute_budget_ixs),
            setup_instructions: Some(setup_ixs?),
            swap_instruction: swap_ix,
            cleanup_instructions: Some(cleanup_ixs?),
            address_lookup_table_addresses: Some(alts.iter().map(|x| x.key.to_string()).collect()),
        });

        Ok(Json(json_response))
    }

    async fn handler() -> Html<&'static str> {
        Html("マンゴールーター")
    }

    async fn liquidity_handler(
        liquidity_provider: LiquidityProviderArcRw,
        Form(input): Form<LiquidityRequest>,
    ) -> Result<Json<Value>, AppError> {
        let mut result = HashMap::new();
        let reader = liquidity_provider.read().unwrap();

        for mint_str in input.mints.split(",") {
            let mint_str = mint_str.trim().to_string();
            let mint = Pubkey::from_str(&mint_str)?;
            result.insert(
                mint_str,
                reader.get_total_liquidity_in_dollars(mint).unwrap_or(0.0),
            );
        }

        drop(reader);
        let json_response = serde_json::json!(LiquidityResponse { liquidity: result });

        Ok(Json(json_response))
    }

    fn extract_client_key(headers: &HeaderMap) -> &str {
        if let Some(client_key) = headers.get("x-client-key") {
            client_key.to_str().unwrap_or("invalid")
        } else {
            "unknown"
        }
    }

    fn setup_router<
        TRouteProvider: RouteProvider + Send + Sync + 'static,
        THashProvider: HashProvider + Send + Sync + 'static,
        TAltProvider: AltProvider + Send + Sync + 'static,
        TAccountProvider: AccountProvider + Send + Sync + 'static,
        TIxBuilder: SwapInstructionsBuilder + Send + Sync + 'static,
    >(
        address_lookup_tables: Vec<String>,
        route_provider: Arc<TRouteProvider>,
        hash_provider: Arc<THashProvider>,
        alt_provider: Arc<TAltProvider>,
        live_account_provider: Arc<TAccountProvider>,
        liquidity_provider: LiquidityProviderArcRw,
        ix_builder: Arc<TIxBuilder>,
        reprice_probability: f64,
    ) -> anyhow::Result<Router<()>> {
        metrics::HTTP_REQUESTS_FAILED.reset();

        let mut router = Router::new();
        let cors = CorsLayer::new()
            .allow_methods(AllowMethods::any())
            .allow_headers(AllowHeaders::any())
            .allow_origin(Any);

        router = router.route("/", routing::get(Self::handler));

        let lp = liquidity_provider.clone();
        router = router.route(
            "/liquidity",
            routing::get(move |form| Self::liquidity_handler(lp, form)),
        );

        let alt = address_lookup_tables.clone();
        let rp = route_provider.clone();
        let hp = hash_provider.clone();
        let altp = alt_provider.clone();
        let lap = live_account_provider.clone();
        let ixb = ix_builder.clone();
        router = router.route(
            "/quote",
            routing::get(move |headers, form| async move {
                let client_key = Self::extract_client_key(&headers);
                let timer = metrics::HTTP_REQUEST_TIMING
                    .with_label_values(&["quote", client_key])
                    .start_timer();

                let response =
                    Self::quote_handler(alt, rp, hp, altp, lap, ixb, reprice_probability, form)
                        .await;

                match response {
                    Ok(_) => {
                        timer.observe_duration();
                        metrics::HTTP_REQUESTS_TOTAL
                            .with_label_values(&["quote", client_key])
                            .inc();
                    }
                    Err(_) => {
                        metrics::HTTP_REQUESTS_FAILED
                            .with_label_values(&["quote", client_key])
                            .inc();
                    }
                }
                response
            }),
        );

        let alt = address_lookup_tables.clone();
        let rp = route_provider.clone();
        let hp = hash_provider.clone();
        let altp = alt_provider.clone();
        let lap = live_account_provider.clone();
        let ixb = ix_builder.clone();
        router = router.route(
            "/swap",
            routing::post(move |headers, query, form| async move {
                let client_key = Self::extract_client_key(&headers);
                let timer = metrics::HTTP_REQUEST_TIMING
                    .with_label_values(&["swap", client_key])
                    .start_timer();

                let response = Self::swap_handler(
                    alt,
                    rp,
                    hp,
                    altp,
                    lap,
                    ixb,
                    reprice_probability,
                    query,
                    form,
                )
                .await;

                match response {
                    Ok(_) => {
                        timer.observe_duration();
                        metrics::HTTP_REQUESTS_TOTAL
                            .with_label_values(&["swap", client_key])
                            .inc();
                    }
                    Err(_) => {
                        metrics::HTTP_REQUESTS_FAILED
                            .with_label_values(&["swap", client_key])
                            .inc();
                    }
                }
                response
            }),
        );

        let alt = address_lookup_tables.clone();
        let rp = route_provider.clone();
        let altp = alt_provider.clone();
        let lap = live_account_provider.clone();
        let ixb = ix_builder.clone();
        router = router.route(
            "/swap-instructions",
            routing::post(move |headers, query, form| async move {
                let client_key = Self::extract_client_key(&headers);
                let timer = metrics::HTTP_REQUEST_TIMING
                    .with_label_values(&["swap-ix", client_key])
                    .start_timer();

                let response = Self::swap_ix_handler(alt, rp, altp, lap, ixb, query, form).await;

                match response {
                    Ok(_) => {
                        timer.observe_duration();
                        metrics::HTTP_REQUESTS_TOTAL
                            .with_label_values(&["swap-ix", client_key])
                            .inc();
                    }
                    Err(_) => {
                        metrics::HTTP_REQUESTS_FAILED
                            .with_label_values(&["swap-ix", client_key])
                            .inc();
                    }
                }
                response
            }),
        );

        router = router.layer(cors);
        Ok(router)
    }

    async fn load_all_alts<TAltProvider: AltProvider + Send + Sync + 'static>(
        address_lookup_table_addresses: Vec<String>,
        alt_provider: Arc<TAltProvider>,
    ) -> Vec<AddressLookupTableAccount> {
        let mut all_alts = vec![];
        for alt in address_lookup_table_addresses {
            match alt_provider.get_alt(Pubkey::from_str(&alt).unwrap()).await {
                Ok(alt) => all_alts.push(alt),
                Err(_) => {}
            }
        }
        all_alts
    }
}

fn reprice<TAccountProvider: AccountProvider + Send + Sync + 'static>(
    route: &Route,
    account_provider: Arc<TAccountProvider>,
) -> anyhow::Result<u64> {
    let account_provider = account_provider.clone() as AccountProviderView;
    let mut amount = route.in_amount;
    for step in &route.steps {
        let prepared_quote = step.edge.prepare(&account_provider)?;
        let quote = step.edge.quote(&prepared_quote, &account_provider, amount);
        amount = quote?.out_amount;
    }
    Ok(amount)
}
