use crate::edge_updater::{spawn_updater_job, Dex};
use crate::hot_mints::HotMintsCache;
use crate::ix_builder::{SwapInstructionsBuilderImpl, SwapStepInstructionBuilderImpl};
use crate::liquidity::{spawn_liquidity_updater_job, LiquidityProvider};
use crate::path_warmer::spawn_path_warmer_job;
use crate::prometheus_sync::PrometheusSync;
use crate::routing::Routing;
use crate::server::alt_provider::RpcAltProvider;
use crate::server::hash_provider::RpcHashProvider;
use crate::server::http_server::HttpServer;
use crate::server::live_account_provider::LiveAccountProvider;
use crate::server::route_provider::RoutingRouteProvider;
use crate::source::mint_accounts_source::{request_mint_metadata, Token};
use crate::token_cache::{Decimals, TokenCache};
use crate::tx_watcher::spawn_tx_watcher_jobs;
use crate::util::tokio_spawn;
use dex_orca::OrcaDex;
use itertools::chain;
use mango_feeds_connector::chain_data::ChainData;
use mango_feeds_connector::SlotUpdate;
use prelude::*;
use router_config_lib::{string_or_env, AccountDataSourceConfig, Config};
use router_feed_lib::account_write::{AccountOrSnapshotUpdate, AccountWrite};
use router_feed_lib::get_program_account::FeedMetadata;
use router_feed_lib::router_rpc_client::RouterRpcClient;
use router_feed_lib::router_rpc_wrapper::RouterRpcWrapper;
use router_lib::chain_data::ChainDataArcRw;
use router_lib::dex::{
    AccountProviderView, ChainDataAccountProvider, DexInterface, DexSubscriptionMode,
};
use router_lib::mango;
use router_lib::price_feeds::composite::CompositePriceFeed;
use router_lib::price_feeds::price_cache::PriceCache;
use router_lib::price_feeds::price_feed::PriceFeed;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::RpcClient as BlockingRpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use source::geyser;
use std::env;
use std::process::exit;
use std::sync::RwLockWriteGuard;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

mod alt;
mod debug_tools;
mod dex;
pub mod edge;
mod edge_updater;
mod hot_mints;
pub mod ix_builder;
mod liquidity;
mod metrics;
mod mock;
mod path_warmer;
pub mod prelude;
mod prometheus_sync;
pub mod routing;
pub mod routing_objectpool;
pub mod routing_types;
pub mod server;
mod slot_watcher;
mod source;
mod swap;
mod syscallstubs;
mod test_utils;
mod tests;
mod token_cache;
mod tx_watcher;
mod util;
mod utils;

// jemalloc seems to be better at keeping the memory footprint reasonable over
// longer periods of time
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[repr(u8)]
enum RouterVersion {
    // Initial = 0,
    OverestimateAmount = 1,
    // Max 15 as to fit into upper 4 bits of an u8
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() -> anyhow::Result<()> {
    router_feed_lib::utils::tracing_subscriber_init();
    syscallstubs::deactivate_program_logs();
    util::print_git_version();
    util::configure_panic_hook();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Please enter a config file path argument.");
        return Ok(());
    }

    let config = Config::load(&args[1])?;
    let router_version = RouterVersion::OverestimateAmount;

    if config.metrics.output_http {
        let prom_bind_addr = config
            .metrics
            .prometheus_address
            .clone()
            .expect("prometheus_address must be set");
        PrometheusSync::sync(prom_bind_addr);
    }
    let hot_mints = Arc::new(RwLock::new(HotMintsCache::new(&config.hot_mints)));

    let mango_data = match mango::mango_fetcher::fetch_mango_data().await {
        Err(e) => {
            error!("Failed to fetch mango metdata: {}", e);
            None
        }
        Ok(metadata) => Some(metadata),
    };

    let region = env::var("FLY_REGION").unwrap_or("undefined".to_string());
    let region_source_config = config
        .sources
        .clone()
        .into_iter()
        .find(|x| *x.region.as_ref().unwrap_or(&"".to_string()) == region);
    let default_source_config = config
        .sources
        .clone()
        .into_iter()
        .find(|x| x.region.is_none());
    let source_config = region_source_config
        .or(default_source_config)
        .unwrap_or_else(|| panic!("did not find a source config for region {}", region));

    let rpc = build_rpc(&source_config);
    let number_of_accounts_per_gma = source_config.number_of_accounts_per_gma.unwrap_or(100);

    // handle sigint
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

    let (account_write_sender, account_write_receiver) =
        async_channel::unbounded::<AccountOrSnapshotUpdate>();
    let (metadata_write_sender, metadata_write_receiver) =
        async_channel::unbounded::<FeedMetadata>();
    let (slot_sender, slot_receiver) = async_channel::unbounded::<SlotUpdate>();
    let (account_update_sender, _) = broadcast::channel(4 * 1024 * 1024); // TODO this is huge, but init snapshot will completely spam this

    let chain_data = Arc::new(RwLock::new(ChainData::new()));
    start_chaindata_updating(
        chain_data.clone(),
        account_write_receiver,
        slot_receiver,
        account_update_sender.clone(),
        exit_sender.subscribe(),
    );

    let (metadata_update_sender, _) = broadcast::channel(500);
    let metadata_update_sender_clone = metadata_update_sender.clone();
    let metadata_job = tokio_spawn("metadata_relayer", async move {
        loop {
            let msg = metadata_write_receiver.recv().await;
            match msg {
                Ok(msg) => {
                    if metadata_update_sender_clone.send(msg).is_err() {
                        error!("Failed to write metadata update");
                        break;
                    }
                }
                Err(_) => {
                    error!("Failed to receives metadata update");
                    break;
                }
            }
        }
    });

    if let Some(quic_sources) = &source_config.quic_sources {
        info!(
            "quic sources: {}",
            quic_sources
                .iter()
                .map(|c| c.connection_string.clone())
                .collect::<String>()
        );
    }
    if let Some(grpc_sources) = source_config.grpc_sources.clone() {
        info!(
            "grpc sources: {}",
            grpc_sources
                .iter()
                .map(|c| c.connection_string.clone())
                .collect::<String>()
        );
    } else {
        // current grpc source is needed for transaction watcher even if there is quic
        error!("No grpc geyser sources specified");
        exit(-1);
    };

    if config.metrics.output_stdout {
        warn!("metrics output to stdout is not supported yet");
    }
    let (mut price_feed, price_feed_job) = build_price_feed(&config, &exit_sender);

    let (price_cache, price_cache_job) =
        PriceCache::new(exit_sender.subscribe(), price_feed.receiver());

    let path_warming_amounts = config
        .routing
        .path_warming_amounts
        .clone()
        .unwrap_or(vec![100, 1000]);

    let mut orca_config = HashMap::new();
    orca_config.insert(
        "program_id".to_string(),
        "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
    );
    orca_config.insert("program_name".to_string(), "Orca".to_string());
    let mut cropper = HashMap::new();
    cropper.insert(
        "program_id".to_string(),
        "H8W3ctz92svYg6mkn1UtGfu2aQr2fnUFHM1RhScEtQDt".to_string(),
    );
    cropper.insert("program_name".to_string(), "Cropper".to_string());

    let enable_compression = source_config.rpc_support_compression.unwrap_or_default();
    let mut router_rpc = RouterRpcClient {
        rpc: Box::new(RouterRpcWrapper {
            rpc: build_rpc(&source_config),
        }),
    };

    let dexs: Vec<Dex> = [
        dex::generic::build_dex!(
            OrcaDex::initialize(&mut router_rpc, orca_config, enable_compression).await?,
            &mango_data,
            config.orca.enabled,
            config.orca.add_mango_tokens,
            config.orca.take_all_mints,
            &config.orca.mints
        ),
        dex::generic::build_dex!(
            OrcaDex::initialize(&mut router_rpc, cropper, enable_compression).await?,
            &mango_data,
            config.cropper.enabled,
            config.cropper.add_mango_tokens,
            config.cropper.take_all_mints,
            &config.cropper.mints
        ),
        dex::generic::build_dex!(
            dex_saber::SaberDex::initialize(&mut router_rpc, HashMap::new(), enable_compression)
                .await?,
            &mango_data,
            config.saber.enabled,
            config.saber.add_mango_tokens,
            config.saber.take_all_mints,
            &config.saber.mints
        ),
        dex::generic::build_dex!(
            dex_raydium_cp::RaydiumCpDex::initialize(
                &mut router_rpc,
                HashMap::new(),
                enable_compression
            )
            .await?,
            &mango_data,
            config.raydium_cp.enabled,
            config.raydium_cp.add_mango_tokens,
            config.raydium_cp.take_all_mints,
            &config.raydium_cp.mints
        ),
        dex::generic::build_dex!(
            dex_raydium::RaydiumDex::initialize(
                &mut router_rpc,
                HashMap::new(),
                enable_compression
            )
            .await?,
            &mango_data,
            config.raydium.enabled,
            config.raydium.add_mango_tokens,
            config.raydium.take_all_mints,
            &config.raydium.mints
        ),
        dex::generic::build_dex!(
            dex_openbook_v2::OpenbookV2Dex::initialize(
                &mut router_rpc,
                HashMap::new(),
                enable_compression
            )
            .await?,
            &mango_data,
            config.openbook_v2.enabled,
            config.openbook_v2.add_mango_tokens,
            config.openbook_v2.take_all_mints,
            &config.openbook_v2.mints
        ),
        dex::generic::build_dex!(
            dex_infinity::InfinityDex::initialize(
                &mut router_rpc,
                HashMap::new(),
                enable_compression
            )
            .await?,
            &mango_data,
            config.infinity.enabled,
            false,
            true,
            &vec![]
        ),
        dex::generic::build_dex!(
            dex_invariant::InvariantDex::initialize(
                &mut router_rpc,
                HashMap::new(),
                enable_compression
            )
            .await?,
            &mango_data,
            config.invariant.enabled,
            config.invariant.take_all_mints,
            config.invariant.add_mango_tokens,
            &config.invariant.mints
        ),
    ]
    .into_iter()
    .flatten()
    .collect();

    let edges = dexs.iter().flat_map(|x| x.edges()).collect_vec();

    // these are around 380k mints
    let mints: HashSet<Pubkey> = chain!(
        edges.iter().map(|x| x.input_mint),
        edges.iter().map(|x| x.output_mint)
    )
    .collect();
    info!("Using {} mints", mints.len(),);

    let token_cache = {
        let mint_metadata = request_mint_metadata(
            &source_config.rpc_http_url,
            &mints,
            number_of_accounts_per_gma,
        )
        .await;
        let mut data: HashMap<Pubkey, token_cache::Decimals> = HashMap::new();
        for (mint_pubkey, Token { mint, decimals }) in mint_metadata {
            assert_eq!(mint_pubkey, mint);
            data.insert(mint_pubkey, decimals as Decimals);
        }
        TokenCache::new(data)
    };

    let (slot_job, rpc_slot_sender) = slot_watcher::spawn_slot_watcher_job(&source_config);
    let ready_channels = dexs
        .iter()
        .map(|_| async_channel::bounded::<()>(1))
        .collect_vec();

    let chain_data_wrapper =
        Arc::new(ChainDataAccountProvider::new(chain_data.clone())) as AccountProviderView;

    let update_jobs = dexs
        .iter()
        .enumerate()
        .filter_map(|(i, dex)| {
            spawn_updater_job(
                dex,
                &config,
                chain_data_wrapper.clone(),
                token_cache.clone(),
                price_cache.clone(),
                path_warming_amounts.clone(),
                price_feed.register_mint_sender(),
                ready_channels[i].0.clone(),
                rpc_slot_sender.subscribe(),
                account_update_sender.subscribe(),
                metadata_update_sender.subscribe(),
                price_feed.receiver(),
                exit_sender.subscribe(),
            )
        })
        .collect_vec();

    let routing = Arc::new(Routing::new(
        &config,
        path_warming_amounts.clone(),
        edges.clone(),
    ));
    let route_provider = Arc::new(RoutingRouteProvider {
        chain_data: chain_data_wrapper.clone(),
        routing,
        hot_mints: hot_mints.clone(),
        prices: price_cache.clone(),
        tokens: token_cache.clone(),
        config: config.safety_checks.clone().unwrap_or(Default::default()),
    });

    let hash_provider = Arc::new(RpcHashProvider {
        rpc_client: rpc,
        last_update: Default::default(),
    });

    let alt_provider = Arc::new(RpcAltProvider {
        rpc_client: build_rpc(&source_config),
        cache: Default::default(),
    });

    let live_account_provider = Arc::new(LiveAccountProvider {
        rpc_client: build_blocking_rpc(&source_config),
    });

    let ix_builder = Arc::new(SwapInstructionsBuilderImpl::new(
        SwapStepInstructionBuilderImpl {
            chain_data: chain_data_wrapper.clone(),
        },
        router_version as u8,
    ));

    let liquidity_provider = Arc::new(RwLock::new(LiquidityProvider::new(
        token_cache.clone(),
        price_cache.clone(),
    )));
    let liquidity_job = spawn_liquidity_updater_job(
        liquidity_provider.clone(),
        edges.clone(),
        chain_data_wrapper,
        exit_sender.subscribe(),
    );

    let server_job = HttpServer::start(
        route_provider.clone(),
        hash_provider,
        alt_provider,
        live_account_provider,
        liquidity_provider.clone(),
        ix_builder,
        config.clone(),
        exit_sender.subscribe(),
    )
    .await?;

    let filters = dexs
        .iter()
        .flat_map(|x| x.edges_per_pk.keys())
        .copied()
        .chain(
            dexs.iter()
                .filter_map(|x| match x.subscription_mode.clone() {
                    DexSubscriptionMode::Accounts(a) => Some(a),
                    DexSubscriptionMode::Mixed(m) => Some(m.accounts),
                    _ => None,
                })
                .flatten(),
        )
        .collect::<HashSet<_>>();

    debug_tools::set_global_filters(&filters);

    info!(
        "Will only react to account writes for {} account(s)",
        filters.len()
    );

    let subscribed_accounts = dexs
        .iter()
        .flat_map(|x| match &x.subscription_mode {
            DexSubscriptionMode::Accounts(x) => x.clone().into_iter(),
            DexSubscriptionMode::Programs(_) => HashSet::new().into_iter(),
            DexSubscriptionMode::Mixed(m) => m.accounts.clone().into_iter(),
            DexSubscriptionMode::Disabled => HashSet::new().into_iter(),
        })
        .collect();

    let subscribed_programs = dexs
        .iter()
        .flat_map(|x| match &x.subscription_mode {
            DexSubscriptionMode::Disabled => HashSet::new().into_iter(),
            DexSubscriptionMode::Accounts(_) => HashSet::new().into_iter(),
            DexSubscriptionMode::Programs(x) => x.clone().into_iter(),
            DexSubscriptionMode::Mixed(m) => m.programs.clone().into_iter(),
        })
        .collect();
    let subscribed_token_accounts = dexs
        .iter()
        .flat_map(|x| match &x.subscription_mode {
            DexSubscriptionMode::Disabled => HashSet::new().into_iter(),
            DexSubscriptionMode::Accounts(_) => HashSet::new().into_iter(),
            DexSubscriptionMode::Programs(_) => HashSet::new().into_iter(),
            DexSubscriptionMode::Mixed(m) => m.token_accounts_for_owner.clone().into_iter(),
        })
        .collect();

    let ef = exit_sender.subscribe();
    let sc = source_config.clone();
    let account_update_job = tokio_spawn("geyser", async move {
        if sc.grpc_sources.is_none() && sc.quic_sources.is_none() {
            error!("No quic or grpc plugin setup");
        } else {
            geyser::spawn_geyser_source(
                &sc,
                ef,
                account_write_sender,
                metadata_write_sender,
                slot_sender,
                &subscribed_accounts,
                &subscribed_programs,
                &subscribed_token_accounts,
                &filters,
            )
            .await;
        }
    });

    let ef = exit_flag.clone();
    let (tx_sender_job, tx_watcher_job) =
        spawn_tx_watcher_jobs(&config.routing, &source_config, &dexs, &exit_sender, ef);

    let mango_watcher_job = mango::mango_fetcher::spawn_mango_watcher(&mango_data, &config);
    let path_warmer_job = spawn_path_warmer_job(
        &config.routing,
        hot_mints.clone(),
        mango_data.clone(),
        route_provider.clone(),
        token_cache,
        price_cache,
        path_warming_amounts,
        exit_flag.clone(),
    );

    let (ready_sender, ready_receiver) = async_channel::bounded::<()>(1);
    let _ready_watcher_job = tokio::spawn(async move {
        for (_, ready) in ready_channels {
            ready.recv().await.unwrap()
        }

        ready_sender.send(()).await.unwrap();
    });

    let mut jobs: futures::stream::FuturesUnordered<_> = vec![
        server_job.join_handle,
        price_feed_job,
        price_cache_job,
        metadata_job,
        slot_job,
        tx_sender_job,
        tx_watcher_job,
        account_update_job,
        liquidity_job,
    ]
    .into_iter()
    .chain(update_jobs.into_iter())
    .chain(mango_watcher_job.into_iter())
    .chain(path_warmer_job.into_iter())
    .collect();

    loop {
        tokio::select!(
            _ = jobs.next() => {
                error!("A critical job exited, aborting run..");
                exit(-1);
            },
            Ok(_) = ready_receiver.recv() => {
                info!("autobahn-router setup complete");
            },
        );
    }

    // unreachable
}

fn build_price_feed(
    config: &Config,
    exit_sender: &broadcast::Sender<()>,
) -> (Box<dyn PriceFeed>, JoinHandle<()>) {
    let x = CompositePriceFeed::start(config.price_feed.clone(), exit_sender.subscribe());
    (Box::new(x.0) as Box<dyn PriceFeed>, x.1)
}

fn build_rpc(source_config: &AccountDataSourceConfig) -> RpcClient {
    RpcClient::new_with_timeouts_and_commitment(
        string_or_env(source_config.rpc_http_url.clone()),
        Duration::from_secs(source_config.request_timeout_in_seconds.unwrap_or(60)), // request timeout
        CommitmentConfig::confirmed(),
        Duration::from_secs(60), // confirmation timeout
    )
}

fn build_blocking_rpc(source_config: &AccountDataSourceConfig) -> BlockingRpcClient {
    BlockingRpcClient::new_with_timeouts_and_commitment(
        string_or_env(source_config.rpc_http_url.clone()),
        Duration::from_secs(source_config.request_timeout_in_seconds.unwrap_or(60)), // request timeout
        CommitmentConfig::confirmed(),
        Duration::from_secs(60), // confirmation timeout
    )
}

fn start_chaindata_updating(
    chain_data: ChainDataArcRw,
    account_writes: async_channel::Receiver<AccountOrSnapshotUpdate>,
    slot_updates: async_channel::Receiver<SlotUpdate>,
    account_update_sender: broadcast::Sender<(Pubkey, Pubkey, u64)>,
    mut exit: broadcast::Receiver<()>,
) -> JoinHandle<()> {
    use mango_feeds_connector::chain_data::SlotData;

    tokio_spawn("chain_data", async move {
        let mut most_recent_seen_slot = 0;

        loop {
            tokio::select! {
                _ = exit.recv() => {
                    info!("shutting down chaindata update task");
                    break;
                }
                res = account_writes.recv() => {
                    let Ok(update) = res
                    else {
                        warn!("account write channel err {res:?}");
                        continue;
                    };

                    let mut writer = chain_data.write().unwrap();
                    handle_updated_account(&mut most_recent_seen_slot, &mut writer, update, &account_update_sender);

                    let mut batchsize: u32 = 0;
                    let started_at = Instant::now();
                    'batch_loop: while let Ok(update) = account_writes.try_recv() {
                        batchsize += 1;

                        handle_updated_account(&mut most_recent_seen_slot, &mut writer, update, &account_update_sender);

                        // budget for microbatch
                        if batchsize > 10 || started_at.elapsed() > Duration::from_micros(500) {
                            break 'batch_loop;
                        }
                    }
                }
                res = slot_updates.recv() => {
                    let Ok(slot_update) = res
                    else {
                        warn!("slot channel err {res:?}");
                        continue;
                    };

                    debug!("chain_data updater got slot: {} ({:?}) -- channel sizes: {} {}", slot_update.slot, slot_update.status,
                    slot_updates.len(), account_writes.len());

                    chain_data.write().unwrap().update_slot(SlotData {
                        slot: slot_update.slot,
                        parent: slot_update.parent,
                        status: slot_update.status,
                        chain: 0,
                    });

                    // TODO: slot updates can significantly affect state, do we need to track what needs to be updated
                    // when switching to a different fork?
                }
                // TODO: update Clock Sysvar
            }
        }
    })
}

fn handle_updated_account(
    most_recent_seen_slot: &mut u64,
    chain_data: &mut RwLockWriteGuard<ChainData>,
    update: AccountOrSnapshotUpdate,
    account_update_sender: &broadcast::Sender<(Pubkey, Pubkey, u64)>,
) {
    use mango_feeds_connector::chain_data::AccountData;
    use solana_sdk::account::WritableAccount;
    use solana_sdk::clock::Epoch;

    fn one_update(
        most_recent_seen_slot: &mut u64,
        chain_data: &mut RwLockWriteGuard<ChainData>,
        account_update_sender: &broadcast::Sender<(Pubkey, Pubkey, u64)>,
        account_write: AccountWrite,
    ) {
        chain_data.update_account(
            account_write.pubkey,
            AccountData {
                slot: account_write.slot,
                write_version: account_write.write_version,
                account: WritableAccount::create(
                    account_write.lamports,
                    account_write.data,
                    account_write.owner,
                    account_write.executable,
                    account_write.rent_epoch as Epoch,
                ),
            },
        );

        if *most_recent_seen_slot != account_write.slot {
            debug!(
                "new slot seen: {} // chain_data.newest_processed_slot: {}",
                account_write.slot,
                chain_data.newest_processed_slot()
            );
            *most_recent_seen_slot = account_write.slot;
        }

        // ignore failing sends when there are no receivers
        let _err = account_update_sender.send((
            account_write.pubkey,
            account_write.owner,
            account_write.slot,
        ));
    }

    match update {
        AccountOrSnapshotUpdate::AccountUpdate(account_write) => one_update(
            most_recent_seen_slot,
            chain_data,
            account_update_sender,
            account_write,
        ),
        AccountOrSnapshotUpdate::SnapshotUpdate(snapshot) => {
            for account_write in snapshot {
                one_update(
                    most_recent_seen_slot,
                    chain_data,
                    account_update_sender,
                    account_write,
                )
            }
        }
    }
}
