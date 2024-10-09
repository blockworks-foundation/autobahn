use crate::edge::Edge;
use crate::metrics;
use crate::token_cache::TokenCache;
use crate::util::tokio_spawn;
use anchor_spl::token::spl_token;
use itertools::Itertools;
use router_config_lib::Config;
use router_feed_lib::get_program_account::FeedMetadata;
use router_lib::dex::{AccountProviderView, DexSubscriptionMode};
use router_lib::price_feeds::price_cache::PriceCache;
use router_lib::price_feeds::price_feed::PriceUpdate;
use solana_program::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct Dex {
    pub name: String,
    /// reference edges by the subscribed_pks so they can be updated on account change
    pub edges_per_pk: HashMap<Pubkey, Vec<Arc<Edge>>>,
    /// in case the program has too many accounts it could overload the rpc subscription
    /// it can be easier to subscribe to the program id directly
    pub subscription_mode: DexSubscriptionMode,
}

impl Dex {
    pub fn edges(&self) -> Vec<Arc<Edge>> {
        let edges: Vec<Arc<Edge>> = self
            .edges_per_pk
            .clone()
            .into_iter()
            .map(|x| x.1)
            .flatten()
            .sorted_by_key(|x| x.unique_id())
            .unique_by(|x| x.unique_id())
            .collect();
        edges
    }
}

#[derive(Default)]
struct EdgeUpdaterState {
    pub is_ready: bool,
    pub latest_slot_pending: u64,
    pub latest_slot_processed: u64,
    pub slot_excessive_lagging_since: Option<Instant>,
    pub dirty_prices: bool,
    pub received_account: HashSet<Pubkey>,
    pub dirty_programs: HashSet<Pubkey>,
    pub dirty_token_accounts_for_owners: bool,
    pub dirty_edges: HashMap<(Pubkey, Pubkey), Arc<Edge>>,
    pub edges_per_mint: HashMap<Pubkey, Vec<Arc<Edge>>>,
}

struct EdgeUpdater {
    dex: Dex,
    chain_data: AccountProviderView,
    token_cache: TokenCache,
    price_cache: PriceCache,
    ready_sender: async_channel::Sender<()>,
    register_mint_sender: async_channel::Sender<Pubkey>,
    state: EdgeUpdaterState,
    config: Config,
    path_warming_amounts: Vec<u64>,
}

pub fn spawn_updater_job(
    dex: &Dex,
    config: &Config,
    chain_data: AccountProviderView,
    token_cache: TokenCache,
    price_cache: PriceCache,
    path_warming_amounts: Vec<u64>,
    register_mint_sender: async_channel::Sender<Pubkey>,
    ready_sender: async_channel::Sender<()>,
    mut slot_updates: broadcast::Receiver<u64>,
    mut account_updates: broadcast::Receiver<(Pubkey, u64)>,
    mut metadata_updates: broadcast::Receiver<FeedMetadata>,
    mut price_updates: broadcast::Receiver<PriceUpdate>,
    mut exit: broadcast::Receiver<()>,
) -> Option<JoinHandle<()>> {
    let dex = dex.clone();

    let config = config.clone();
    let edges = dex.edges();

    let mut edges_per_mint = HashMap::<Pubkey, Vec<Arc<Edge>>>::default();
    for edge in &edges {
        edges_per_mint
            .entry(edge.input_mint)
            .or_default()
            .push(edge.clone());
        edges_per_mint
            .entry(edge.output_mint)
            .or_default()
            .push(edge.clone());
    }

    match &dex.subscription_mode {
        DexSubscriptionMode::Accounts(x) => info!(
            dex_name = dex.name,
            accounts_count = x.len(),
            "subscribing to accounts"
        ),
        DexSubscriptionMode::Programs(x) => info!(
            dex_name = dex.name,
            programs = x.iter().map(|p| p.to_string()).join(", "),
            "subscribing to programs"
        ),
        DexSubscriptionMode::Disabled => {
            debug!(dex_name = dex.name, "disabled");
            let _ = ready_sender.try_send(());
            return None;
        }
        DexSubscriptionMode::Mixed(m) => info!(
            dex_name = dex.name,
            accounts_count = m.accounts.len(),
            programs = m.programs.iter().map(|p| p.to_string()).join(", "),
            token_accounts_for_owner = m
                .token_accounts_for_owner
                .iter()
                .map(|p| p.to_string())
                .join(", "),
            "subscribing to mixed mode"
        ),
    };

    let init_timeout_in_seconds = config.snapshot_timeout_in_seconds.unwrap_or(60 * 5);
    let init_timeout = Instant::now() + Duration::from_secs(init_timeout_in_seconds);
    let listener_job = tokio_spawn(format!("edge_updater_{}", dex.name).as_str(), async move {
        let mut updater = EdgeUpdater {
            dex,
            chain_data,
            token_cache,
            price_cache,
            register_mint_sender,
            ready_sender,
            config,
            state: EdgeUpdaterState {
                edges_per_mint,
                ..EdgeUpdaterState::default()
            },
            path_warming_amounts,
        };

        let mut refresh_one_interval = tokio::time::interval(Duration::from_millis(10));
        refresh_one_interval.tick().await;

        'drain_loop: loop {
            tokio::select! {
                _ = exit.recv() => {
                    info!("shutting down {} update task", updater.dex.name);
                    break;
                }
                slot = slot_updates.recv() => {
                    updater.detect_and_handle_slot_lag(slot);
                }
                res = metadata_updates.recv() => {
                    updater.on_metadata_update(res);
                }
                res = account_updates.recv() => {
                    if !updater.invalidate_one(res) {
                        break 'drain_loop;
                    }

                    let mut batchsize: u32 = 0;
                    let started_at = Instant::now();
                    'batch_loop: while let Ok(res) = account_updates.try_recv() {
                        batchsize += 1;
                        if !updater.invalidate_one(Ok(res)) {
                            break 'drain_loop;
                        }

                        // budget for microbatch
                        if batchsize > 10 || started_at.elapsed() > Duration::from_micros(500) {
                            break 'batch_loop;
                        }
                    }
                },
                Ok(price_upd) = price_updates.recv() => {
                    if let Some(impacted_edges) = updater.state.edges_per_mint.get(&price_upd.mint) {
                        for edge in impacted_edges {
                            updater.state.dirty_edges.insert(edge.unique_id(), edge.clone());
                        }
                    };
                },
                _ = refresh_one_interval.tick() => {
                    if !updater.state.is_ready && init_timeout < Instant::now() {
                        error!("Failed to init '{}' before timeout", updater.dex.name);
                        break;
                    }

                    updater.refresh_some();
                }
            }
        }

        error!("Edge updater {} job exited..", updater.dex.name);
        // send this to unblock the code in front of the exit handler
        let _ = updater.ready_sender.try_send(());
    });

    Some(listener_job)
}

impl EdgeUpdater {
    fn detect_and_handle_slot_lag(&mut self, slot: Result<u64, RecvError>) {
        let state = &mut self.state;
        if state.latest_slot_processed == 0 {
            return;
        }
        if let Ok(slot) = slot {
            let lag = slot as i64 - state.latest_slot_processed as i64;
            if lag <= 0 {
                return;
            }
            debug!(
                state.latest_slot_processed,
                state.latest_slot_pending, slot, lag, self.dex.name, "metrics"
            );

            metrics::GRPC_TO_EDGE_SLOT_LAG
                .with_label_values(&[&self.dex.name])
                .set(lag);

            let max_lag = self.config.routing.slot_excessive_lag.unwrap_or(300);
            let max_lag_duration = Duration::from_secs(
                self.config
                    .routing
                    .slot_excessive_lag_max_duration_secs
                    .unwrap_or(60),
            );

            if lag as u64 >= max_lag {
                match state.slot_excessive_lagging_since {
                    None => state.slot_excessive_lagging_since = Some(Instant::now()),
                    Some(since) => {
                        if since.elapsed() > max_lag_duration {
                            panic!(
                                "Lagging a lot {} for more than {}s, for dex {}..",
                                lag,
                                max_lag_duration.as_secs(),
                                self.dex.name,
                            );
                        }
                    }
                }
                return;
            } else if state.slot_excessive_lagging_since.is_some() {
                state.slot_excessive_lagging_since = None;
            }
        }
    }

    // called once after startup
    #[tracing::instrument(skip_all, level = "trace")]
    fn on_ready(&self) {
        let mut mints = HashSet::new();
        for edge in self.dex.edges() {
            mints.insert(edge.input_mint);
            mints.insert(edge.output_mint);
        }

        info!(
            "Received all accounts needed for {} [mints count={}]",
            self.dex.name,
            mints.len()
        );

        for mint in mints {
            match self.register_mint_sender.try_send(mint) {
                Ok(_) => {}
                Err(_) => warn!("Failed to register mint '{}' for price update", mint),
            }
        }

        let _ = self.ready_sender.try_send(());
    }

    fn on_metadata_update(&mut self, res: Result<FeedMetadata, RecvError>) {
        let state = &mut self.state;
        match res {
            Ok(v) => match v {
                FeedMetadata::InvalidAccount(key) => {
                    state.received_account.insert(key);
                    self.check_readiness();
                }
                FeedMetadata::SnapshotStart(_) => {}
                FeedMetadata::SnapshotEnd(x) => {
                    if let Some(x) = x {
                        if x == spl_token::ID {
                            // TODO Handle multiples owners
                            state.dirty_token_accounts_for_owners = true;
                        } else {
                            state.dirty_programs.insert(x);
                        }
                        self.check_readiness();
                    }
                }
            },
            Err(e) => {
                warn!(
                    "Error on metadata update channel in {} update task {:?}",
                    self.dex.name, e
                );
            }
        }
    }

    fn invalidate_one(&mut self, res: Result<(Pubkey, u64), RecvError>) -> bool {
        let state = &mut self.state;
        let (pk, slot) = match res {
            Ok(v) => v,
            Err(broadcast::error::RecvError::Closed) => {
                error!("account update channel closed unexpectedly");
                return false;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(
                    "lagged {n} on account update channel in {} update task",
                    self.dex.name
                );
                return true;
            }
        };

        if let Some(impacted_edges) = self.dex.edges_per_pk.get(&pk) {
            for edge in impacted_edges {
                state.dirty_edges.insert(edge.unique_id(), edge.clone());
            }
        };

        state.received_account.insert(pk);
        if state.latest_slot_pending < slot {
            state.latest_slot_pending = slot;
        }

        self.check_readiness();

        return true;
    }

    fn check_readiness(&mut self) {
        let state = &mut self.state;

        if state.is_ready {
            return;
        }

        match &self.dex.subscription_mode {
            DexSubscriptionMode::Accounts(accounts) => {
                state.is_ready = state.received_account.is_superset(&accounts);
            }
            DexSubscriptionMode::Disabled => {}
            DexSubscriptionMode::Programs(programs) => {
                state.is_ready = state.dirty_programs.is_superset(&programs);
            }
            DexSubscriptionMode::Mixed(m) => {
                state.is_ready = state.received_account.is_superset(&m.accounts)
                    && state.dirty_programs.is_superset(&m.programs)
                    && (state.dirty_token_accounts_for_owners
                        || m.token_accounts_for_owner.is_empty());
            }
        }

        if state.is_ready {
            self.on_ready();
        }
    }

    fn refresh_some(&mut self) {
        let state = &mut self.state;
        if state.dirty_edges.is_empty() || !state.is_ready {
            return;
        }

        let started_at = Instant::now();
        let mut refreshed_edges = vec![];

        for (unique_id, edge) in &state.dirty_edges {
            edge.update(
                &self.chain_data,
                &self.token_cache,
                &self.price_cache,
                &self.path_warming_amounts,
            );
            refreshed_edges.push(unique_id.clone());

            // Do not process for too long or we could miss update in account write queue
            if started_at.elapsed() > Duration::from_millis(100) {
                break;
            }
        }
        for unique_id in &refreshed_edges {
            state.dirty_edges.remove(&unique_id);
        }

        state.latest_slot_processed = state.latest_slot_pending;

        if started_at.elapsed() > Duration::from_millis(100) {
            info!(
                "{} - refresh {} - took - {:?}",
                self.dex.name,
                refreshed_edges.len(),
                started_at.elapsed()
            )
        }
    }
}
