use crate::debug_tools;
use crate::metrics;
use crate::prelude::*;
use crate::routing_objectpool::RoutingObjectPools;
use mango_feeds_connector::chain_data::AccountData;
use ordered_float::NotNan;
use router_config_lib::Config;
use router_lib::dex::SwapMode;
use router_lib::dex::{AccountProviderView, DexEdge};
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::time::{Duration, Instant};
use std::u64;
use thiserror::Error;
use tracing::Level;

use crate::routing_types::*;

#[derive(Error, Debug)]
pub enum RoutingError {
    #[error("unsupported input mint {0:?}")]
    UnsupportedInputMint(Pubkey),
    #[error("unsupported output mint {0:?}")]
    UnsupportedOutputMint(Pubkey),
    #[error("no path between {0:?} and {1:?}")]
    NoPathBetweenMintPair(Pubkey, Pubkey),
    #[error("could not compute out amount")]
    CouldNotComputeOut,
}

fn best_price_paths_depth_search<F>(
    input_node: MintNodeIndex,
    amount: u64,
    max_path_length: usize,
    max_accounts: usize,
    out_edges_per_node: &MintVec<Vec<EdgeWithNodes>>,
    // caller must provide this complying the size rules (see below)
    best_paths_by_node_prealloc: &mut MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>,
    // caller must provide this complying the size rules (see below)
    best_by_node_prealloc: &mut Vec<BestVec3>,

    edge_price: F,
    hot_mints: &HashSet<MintNodeIndex>,
    avoid_cold_mints: bool,
    swap_mode: SwapMode,
) -> anyhow::Result<MintVec<Vec<(f64, Vec<EdgeWithNodes>)>>>
where
    F: Fn(EdgeIndex, u64) -> Option<EdgeInfo>,
{
    debug!(input_node = input_node.idx_raw(), amount, "finding path");

    let max_accounts = max_accounts.min(40);

    if tracing::event_enabled!(Level::TRACE) {
        let count = count_edges(out_edges_per_node);
        trace!(count, "available edges");
        trace!(
            count = out_edges_per_node[0.into()].len(),
            "available edges out of 0"
        );
    };

    let mut path = Vec::new();

    // mints -> best_paths -> edges; best_paths are some 64; edges are some 4
    let mut best_paths_by_node: &mut MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>> =
        best_paths_by_node_prealloc;

    // Best amount received for token/account_size
    // 3 = number of path kept
    // 8 = (64/8) (max accounts/bucket_size)
    assert_eq!(
        best_by_node_prealloc.len(),
        8 * out_edges_per_node.len(),
        "best_by_node_prealloc len error"
    );
    assert!(
        best_by_node_prealloc.iter().all(|v| v.len() == 3),
        "best_by_node_prealloc vector items length error"
    );
    let mut best_by_node = best_by_node_prealloc;

    let mut stats = vec![0; 2];

    // initialize best_paths_by_node and best_by_node using direct path
    {
        let current_account_count = 0;
        let in_amount = amount as f64;
        for out_edge in &out_edges_per_node[input_node] {
            try_append_to_best_results(
                &mut best_paths_by_node,
                &mut path,
                &mut best_by_node,
                &mut stats,
                in_amount,
                max_accounts,
                current_account_count,
                &edge_price,
                &out_edge,
                swap_mode,
            );
        }
    }

    // Depth first search
    walk(
        &mut best_paths_by_node,
        &mut path,
        &mut best_by_node,
        &mut stats,
        amount as f64,
        input_node,
        max_path_length,
        max_accounts,
        0,
        out_edges_per_node,
        &edge_price,
        hot_mints,
        avoid_cold_mints,
        swap_mode,
    );

    let good_paths = best_paths_by_node
        .iter()
        .map(|best_paths| {
            best_paths
                .into_iter()
                .filter_map(|(out, edges)| {
                    // did not manage to .iter_into()
                    let edges = edges.clone();
                    match swap_mode {
                        SwapMode::ExactIn => {
                            out.is_sign_positive().then_some((out.into_inner(), edges))
                        }
                        SwapMode::ExactOut => out.is_finite().then_some((out.into_inner(), edges)),
                    }
                })
                .collect_vec()
        })
        .collect_vec();

    debug!(
        input_node = input_node.idx_raw(),
        amount,
        probed = stats[0],
        skipped = stats[1],
        "done"
    );

    Ok(good_paths.into())
}

fn walk<F2>(
    best_paths_by_node: &mut MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>,
    path: &mut Vec<EdgeWithNodes>,
    // indexed by "bucket" which is neither Node nor Edge Index
    best_by_node: &mut Vec<BestVec3>,
    stats: &mut Vec<u64>,
    in_amount: f64,
    input_node: MintNodeIndex,
    max_path_length: usize,
    max_accounts: usize,
    current_account_count: usize,
    out_edges_per_node: &MintVec<Vec<EdgeWithNodes>>,
    edge_price_fn: &F2,
    hot_mints: &HashSet<MintNodeIndex>,
    avoid_cold_mints: bool,
    swap_mode: SwapMode,
) where
    F2: Fn(EdgeIndex, u64) -> Option<EdgeInfo>,
{
    if max_path_length == 0
        || max_accounts < (current_account_count + 4)
        || in_amount.is_nan()
        || in_amount.is_sign_negative()
    {
        return;
    }

    stats[0] += 1;

    for out_edge in &out_edges_per_node[input_node] {
        // no cycles
        if path
            .iter()
            .any(|step| step.source_node == out_edge.target_node)
        {
            continue;
        }

        let Some((edge_info, out_amount)) = try_append_to_best_results(
            best_paths_by_node,
            path,
            best_by_node,
            stats,
            in_amount,
            max_accounts,
            current_account_count,
            edge_price_fn,
            &out_edge,
            swap_mode,
        ) else {
            continue;
        };

        // Stop depth search when encountering a cold mint
        if avoid_cold_mints && hot_mints.len() > 0 && !hot_mints.contains(&out_edge.source_node) {
            stats[1] += 1;
            continue;
        }

        path.push(out_edge.clone());

        walk(
            best_paths_by_node,
            path,
            best_by_node,
            stats,
            out_amount,
            out_edge.target_node,
            max_path_length - 1,
            max_accounts,
            current_account_count + edge_info.accounts,
            out_edges_per_node,
            edge_price_fn,
            hot_mints,
            avoid_cold_mints,
            swap_mode,
        );

        path.pop();
    }
}

fn try_append_to_best_results<F2>(
    best_paths_by_node: &mut MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>,
    path: &Vec<EdgeWithNodes>,
    best_by_node: &mut Vec<BestVec3>,
    stats: &mut Vec<u64>,
    in_amount: f64,
    max_accounts: usize,
    current_account_count: usize,
    edge_price_fn: &F2,
    out_edge: &EdgeWithNodes,
    swap_mode: SwapMode,
) -> Option<(EdgeInfo, f64)>
where
    F2: Fn(EdgeIndex, u64) -> Option<EdgeInfo>,
{
    let Some(edge_info) = edge_price_fn(out_edge.edge, in_amount as u64) else {
        return None;
    };
    if current_account_count + edge_info.accounts > max_accounts {
        return None;
    }

    let out_amount = in_amount * edge_info.price;

    let best_paths = &mut best_paths_by_node[out_edge.target_node];
    let worst = best_paths
        .last()
        .map(|(p, _)| p.into_inner())
        .unwrap_or(match swap_mode {
            SwapMode::ExactIn => f64::NEG_INFINITY,
            SwapMode::ExactOut => f64::INFINITY,
        });

    if (swap_mode == SwapMode::ExactOut && out_amount < worst)
        || (swap_mode == SwapMode::ExactIn && out_amount > worst)
    {
        replace_worst_path(path, out_edge, out_amount, best_paths, swap_mode);
    }

    let account_bucket = (current_account_count + edge_info.accounts).div_euclid(8);
    let target_node_bucket = out_edge.target_node.idx_raw() * 8 + account_bucket as u32;
    let target_node_bucket = target_node_bucket as usize;
    let values = &best_by_node[target_node_bucket];
    let smaller_value_index = get_already_seen_worst_kept_output(values);

    if values[smaller_value_index] > out_amount {
        stats[1] += 1;
        return None;
    }

    best_by_node[target_node_bucket][smaller_value_index] = out_amount;
    Some((edge_info, out_amount))
}

fn get_already_seen_worst_kept_output(values: &BestVec3) -> usize {
    values
        .iter()
        .position_min_by_key(|v| v.floor() as u64)
        .expect("can't find min index")
}

fn replace_worst_path(
    current_path: &Vec<EdgeWithNodes>,
    added_hop: &EdgeWithNodes,
    out_amount: f64,
    best_paths: &mut Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>,
    swap_mode: SwapMode,
) {
    // TODO bad for perf - try to find a better solution than this
    let already_exists = path_already_in(current_path, added_hop, out_amount, best_paths);

    if !already_exists {
        let mut added_path = current_path.clone();
        added_path.push(added_hop.clone());

        best_paths.pop();
        best_paths.push((NotNan::new(out_amount).unwrap(), added_path));
        match swap_mode {
            SwapMode::ExactIn => best_paths.sort_by_key(|(p, _)| std::cmp::Reverse(*p)),
            SwapMode::ExactOut => best_paths.sort_by_key(|(p, _)| *p),
        };
    }
}

fn path_already_in(
    current_path: &Vec<EdgeWithNodes>,
    added_hop: &EdgeWithNodes,
    out_amount: f64,
    best_paths: &Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>,
) -> bool {
    let mut already_exists = false;
    'outer: for existing_path in best_paths.iter() {
        if existing_path.0 != NotNan::new(out_amount).unwrap() {
            continue;
        }
        if existing_path.1.len() != (current_path.len() + 1) {
            continue;
        }

        for i in 0..current_path.len() {
            if existing_path.1[i].edge != current_path[i].edge {
                continue 'outer;
            }
        }

        if existing_path.1[existing_path.1.len() - 1].edge != added_hop.edge {
            continue;
        }

        already_exists = true;
        break;
    }
    already_exists
}

fn count_edges(edges: &[Vec<EdgeWithNodes>]) -> u64 {
    let mut set = HashSet::new();
    for node in 0..edges.len() {
        for edge in &edges[node] {
            set.insert(edge.edge);
        }
    }
    return set.len() as u64;
}

struct PathDiscoveryCacheEntry {
    timestamp_millis: u64,
    in_amount: f64,
    max_account: usize,
    // mint x mint
    edges: Vec<Vec<EdgeIndex>>,
}

struct PathDiscoveryCache {
    cache: HashMap<(MintNodeIndex, MintNodeIndex, SwapMode), Vec<PathDiscoveryCacheEntry>>,
    last_expire_timestamp_millis: u64,
    max_age_millis: u64,
}

impl PathDiscoveryCache {
    fn expire_old(&mut self) {
        let now = millis_since_epoch();
        if now - self.last_expire_timestamp_millis < 1000 {
            return;
        }
        self.cache.retain(|_k, entries| {
            entries.retain(|entry| {
                entry.timestamp_millis > now || now - entry.timestamp_millis < self.max_age_millis
            });
            !entries.is_empty()
        });
        self.last_expire_timestamp_millis = now;
    }

    fn expire_old_by_key(max_age_millis: u64, entries: &mut Vec<PathDiscoveryCacheEntry>) {
        let now = millis_since_epoch();
        entries.retain(|entry| {
            entry.timestamp_millis > now || now - entry.timestamp_millis < max_age_millis
        });
    }

    fn insert(
        &mut self,
        from: MintNodeIndex,
        to: MintNodeIndex,
        swap_mode: SwapMode,
        in_amount: u64,
        max_accounts: usize,
        timestamp_millis: u64,
        mut edges: Vec<Vec<EdgeIndex>>,
    ) {
        // some 3-4
        trace!(
            "insert entry into discovery cache with edges cardinality of {}",
            edges.len()
        );

        let max_accounts_bucket = Self::compute_account_bucket(max_accounts);
        let entry = self.cache.entry((from, to, swap_mode)).or_default();

        // try to reduce memory footprint...
        for path in &mut edges {
            path.shrink_to_fit();
        }
        edges.shrink_to_fit();

        let new_elem = PathDiscoveryCacheEntry {
            timestamp_millis,
            in_amount: in_amount as f64,
            max_account: max_accounts_bucket,
            edges,
        };

        let pos = entry
            .binary_search_by_key(&(in_amount, max_accounts_bucket), |x| {
                (x.in_amount.round() as u64, x.max_account)
            })
            .unwrap_or_else(|e| e);

        // Replace if it already exists (instead of doubling cache size with old and new path)
        if pos < entry.len()
            && entry[pos].max_account == max_accounts_bucket
            && entry[pos].in_amount.round() as u64 == in_amount
        {
            entry[pos] = new_elem;
            return;
        }

        entry.insert(pos, new_elem);
    }

    fn get(
        &mut self,
        from: MintNodeIndex,
        to: MintNodeIndex,
        swap_mode: SwapMode,
        in_amount: u64,
        max_accounts: usize,
    ) -> (Option<&Vec<Vec<EdgeIndex>>>, Option<&Vec<Vec<EdgeIndex>>>) {
        let in_amount = in_amount as f64;
        let max_accounts_bucket = Self::compute_account_bucket(max_accounts);
        let Some(entries) = self.cache.get_mut(&(from, to, swap_mode)) else {
            // cache miss
            metrics::PATH_DISCOVERY_CACHE_MISSES.inc();
            return (None, None);
        };
        metrics::PATH_DISCOVERY_CACHE_HITS.inc();

        Self::expire_old_by_key(self.max_age_millis, entries);

        let (mut lower, mut upper) = (None, None);
        for entry in entries {
            if entry.max_account != max_accounts_bucket {
                continue;
            }
            if entry.in_amount <= in_amount {
                lower = Some(&entry.edges);
            }
            if entry.in_amount > in_amount {
                upper = Some(&entry.edges);
                break;
            }
        }

        (lower, upper)
    }

    fn invalidate(&mut self, from: MintNodeIndex, to: MintNodeIndex, max_accounts: usize) {
        let max_accounts_bucket = Self::compute_account_bucket(max_accounts);
        let Some(entries) = self.cache.get_mut(&(from, to, SwapMode::ExactIn)) else {
            return;
        };

        entries.retain(|x| x.max_account != max_accounts_bucket);

        let Some(entries) = self.cache.get_mut(&(from, to, SwapMode::ExactOut)) else {
            return;
        };
        entries.retain(|x| x.max_account != max_accounts_bucket);
    }

    fn compute_account_bucket(max_accounts: usize) -> usize {
        max_accounts.div_euclid(5)
    }
}

/// global singleton to manage routing
#[allow(dead_code)]
pub struct Routing {
    // indexed by EdgeIndex
    edges: Vec<Arc<Edge>>,

    // indexed by MintNodeIndex
    mints: MintVec<Pubkey>,

    // memory pools for best_price_paths_depth_search
    objectpools: RoutingObjectPools,

    // Preparation for pathfinding
    // mint pubkey -> NodeIndex
    mint_to_index: HashMap<Pubkey, MintNodeIndex>,

    path_discovery_cache: RwLock<PathDiscoveryCache>,

    // Keep pruned edges for a while (speed up searching)
    // first for exact in and second of exact out
    pruned_out_edges_per_mint_index_exact_in: RwLock<(Instant, MintVec<Vec<EdgeWithNodes>>)>,
    pruned_out_edges_per_mint_index_exact_out: RwLock<(Instant, MintVec<Vec<EdgeWithNodes>>)>,
    path_warming_amounts: Vec<u64>,

    // Optim & Heuristics
    overquote: f64,
    max_path_length: usize,
    retain_path_count: usize,
    max_edge_per_pair: usize,
    max_edge_per_cold_pair: usize,
}

impl Routing {
    pub fn new(
        configuration: &Config,
        path_warming_amounts: Vec<u64>,
        edges: Vec<Arc<Edge>>,
    ) -> Self {
        let mints: MintVec<Pubkey> = edges
            .iter()
            .flat_map(|e| [e.input_mint, e.output_mint])
            .unique()
            .collect_vec()
            .into();
        let mint_to_index: HashMap<Pubkey, MintNodeIndex> = mints
            .iter()
            .enumerate()
            .map(|(i, mint_pubkey)| (*mint_pubkey, i.into()))
            .collect();

        info!(
            "Setup routing algorithm with {} edges and {} distinct mints",
            edges.len(),
            mints.len()
        );

        let mint_count = mints.len();
        let retain_path_count = configuration.routing.retain_path_count.unwrap_or(10);

        Self {
            edges,
            mints,
            objectpools: RoutingObjectPools::new(mint_count, retain_path_count),
            mint_to_index,
            path_discovery_cache: RwLock::new(PathDiscoveryCache {
                cache: Default::default(),
                last_expire_timestamp_millis: 0,
                max_age_millis: configuration.routing.path_cache_validity_ms,
            }),
            pruned_out_edges_per_mint_index_exact_in: RwLock::new((
                Instant::now() - Duration::from_secs(3600 * 24),
                MintVec::new_from_prototype(0, vec![]),
            )),
            pruned_out_edges_per_mint_index_exact_out: RwLock::new((
                Instant::now() - Duration::from_secs(3600 * 24),
                MintVec::new_from_prototype(0, vec![]),
            )),
            path_warming_amounts,
            overquote: configuration.routing.overquote.unwrap_or(0.20),
            max_path_length: configuration.routing.max_path_length.unwrap_or(4),
            retain_path_count,
            max_edge_per_pair: configuration.routing.max_edge_per_pair.unwrap_or(8),
            max_edge_per_cold_pair: configuration.routing.max_edge_per_cold_pair.unwrap_or(3),
        }
    }

    /// This should never do anything if path_warming is enabled
    pub fn prepare_pruned_edges_if_not_initialized(
        &self,
        hot_mints: &HashSet<Pubkey>,
        swap_mode: SwapMode,
    ) {
        let reader = match swap_mode {
            SwapMode::ExactIn => self
                .pruned_out_edges_per_mint_index_exact_in
                .read()
                .unwrap(),
            SwapMode::ExactOut => self
                .pruned_out_edges_per_mint_index_exact_out
                .read()
                .unwrap(),
        };

        let need_refresh = reader.1.len() == 0 || reader.0.elapsed() > Duration::from_secs(60 * 15);
        drop(reader);
        if need_refresh {
            self.prepare_pruned_edges_and_cleanup_cache(hot_mints, swap_mode);
        }
    }

    // called per request of criteria match
    #[tracing::instrument(skip_all, level = "trace")]
    pub fn prepare_pruned_edges_and_cleanup_cache(
        &self,
        hot_mints: &HashSet<Pubkey>,
        swap_mode: SwapMode,
    ) {
        debug!("prepare_pruned_edges_and_cleanup_cache started");
        self.path_discovery_cache.write().unwrap().expire_old();

        let (valid_edge_count, out_edges_per_mint_index) = Self::select_best_pools(
            &hot_mints,
            self.max_edge_per_pair,
            self.max_edge_per_cold_pair,
            &self.path_warming_amounts,
            &self.edges,
            &self.mint_to_index,
            swap_mode,
        );

        let mut writer = match swap_mode {
            SwapMode::ExactIn => self
                .pruned_out_edges_per_mint_index_exact_in
                .write()
                .unwrap(),
            SwapMode::ExactOut => self
                .pruned_out_edges_per_mint_index_exact_out
                .write()
                .unwrap(),
        };
        if valid_edge_count > 0 {
            (*writer).0 = Instant::now();
        }
        if !(*writer).1.try_clone_from(&out_edges_per_mint_index) {
            debug!("Failed to use clone_into out_edges_per_mint_index, falling back to slower assignment");
            (*writer).1 = out_edges_per_mint_index;
        }

        debug!("prepare_pruned_edges_and_cleanup_cache done");
    }

    fn compute_price_impact(edge: &Arc<Edge>) -> Option<f64> {
        let state = edge.state.read().unwrap();
        if !state.is_valid() || state.cached_prices.len() < 2 {
            return None;
        }

        let first = state.cached_prices[0].1;
        let last = state.cached_prices[state.cached_prices.len() - 1].1;

        if first == 0.0 || last == 0.0 {
            return None;
        }

        if first < last {
            debug!(
                edge = edge.id.desc(),
                input = debug_tools::name(&edge.id.input_mint()),
                output = debug_tools::name(&edge.id.output_mint()),
                "weird thing happening"
            );

            // Weird, price for bigger amount should be less good than for small amount
            // But it can happen with openbook because of the lot size
            Some(last / first - 1.0)
        } else {
            Some(first / last - 1.0)
        }
    }

    fn select_best_pools(
        hot_mints: &HashSet<Pubkey>,
        max_count_for_hot: usize,
        max_edge_per_cold_pair: usize,
        path_warming_amounts: &Vec<u64>,
        all_edges: &Vec<Arc<Edge>>,
        mint_to_index: &HashMap<Pubkey, MintNodeIndex>,
        swap_mode: SwapMode,
    ) -> (i32, MintVec<Vec<EdgeWithNodes>>) {
        let mut result = HashSet::new();

        // best for exact in theoritically should also be best for exact out
        for (i, _amount) in path_warming_amounts.iter().enumerate() {
            let mut best = HashMap::<(Pubkey, Pubkey), Vec<(EdgeIndex, f64)>>::new();

            for (edge_index, edge) in all_edges.iter().enumerate() {
                trace!("ix:{edge_index} edge:{edge:?}");
                if swap_mode == SwapMode::ExactOut && !edge.supports_exact_out() {
                    continue;
                }

                let edge_index: EdgeIndex = edge_index.into();
                let state = edge.state.read().unwrap();
                trace!("ix:{edge_index} edge:{edge:?} {:?}", state.cached_prices);

                if !state.is_valid() || state.cached_prices.len() < i {
                    continue;
                }
                let price = state.cached_prices[i].1;
                if price.is_nan() || price.is_sign_negative() || price <= 0.000000001 {
                    continue;
                }

                let entry = best.entry((edge.input_mint, edge.output_mint));
                let is_hot = hot_mints.is_empty()
                    || (hot_mints.contains(&edge.input_mint)
                        && hot_mints.contains(&edge.output_mint));
                let max_count = if is_hot {
                    max_count_for_hot
                } else {
                    max_edge_per_cold_pair
                };

                match entry {
                    Occupied(mut e) => {
                        let vec = e.get_mut();
                        if vec.len() == max_count {
                            let should_replace = vec[max_count - 1].1 < price;
                            if should_replace {
                                vec[max_count - 1] = (edge_index, price);
                                vec.sort_unstable_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
                            }
                        } else {
                            vec.push((edge_index, price));
                            vec.sort_unstable_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
                            vec.truncate(max_count);
                        }
                    }
                    Vacant(v) => {
                        v.insert(vec![(edge_index, price)]);
                    }
                }
            }

            let res: Vec<EdgeIndex> = best
                .into_iter()
                .map(|x| x.1)
                .flatten()
                .map(|x| x.0)
                .collect();
            result.extend(res);
        }

        let mut valid_edge_count = 0;
        // TODO how to reuse that? mabye objectpool
        let mut out_edges_per_mint_index: MintVec<Vec<EdgeWithNodes>> =
            MintVec::new_from_prototype(mint_to_index.len(), vec![]);
        let mut lower_price_impact_edge_for_mint_and_direction =
            HashMap::<(MintNodeIndex, bool), (f64, EdgeIndex)>::new();
        let mut has_edge_for_mint = HashSet::new();

        let mut skipped_bad_price_impact = 0;

        for edge_index in result.iter() {
            let edge = &all_edges[edge_index.idx()];
            let in_index = mint_to_index[&edge.input_mint];
            let out_index = mint_to_index[&edge.output_mint];
            let in_key = (in_index, true);
            let out_key = (out_index, false);

            let price_impact = Self::compute_price_impact(&edge).unwrap_or(9999.9999);
            Self::update_lowest_price_impact(
                &mut lower_price_impact_edge_for_mint_and_direction,
                *edge_index,
                in_key,
                price_impact,
            );
            Self::update_lowest_price_impact(
                &mut lower_price_impact_edge_for_mint_and_direction,
                *edge_index,
                out_key,
                price_impact,
            );

            // if price_impact > 0.25 {
            //     skipped_bad_price_impact += 1;
            //     continue;
            // }

            match swap_mode {
                SwapMode::ExactIn => {
                    out_edges_per_mint_index[in_index].push(EdgeWithNodes {
                        source_node: in_index,
                        target_node: out_index,
                        edge: *edge_index,
                    });
                }
                SwapMode::ExactOut => {
                    out_edges_per_mint_index[out_index].push(EdgeWithNodes {
                        source_node: out_index,
                        target_node: in_index,
                        edge: *edge_index,
                    });
                }
            }

            has_edge_for_mint.insert(in_key);
            has_edge_for_mint.insert(out_key);
            valid_edge_count += 1;
        }

        for (key, (_, edge_index)) in lower_price_impact_edge_for_mint_and_direction {
            let has_edge_for_mint_and_direction = has_edge_for_mint.contains(&key);
            if has_edge_for_mint_and_direction {
                continue;
            }
            let edge = &all_edges[edge_index.idx()];
            let in_index = mint_to_index[&edge.input_mint];
            let out_index = mint_to_index[&edge.output_mint];
            let in_key = (in_index, true);
            let out_key = (out_index, false);

            match swap_mode {
                SwapMode::ExactIn => {
                    out_edges_per_mint_index[in_index].push(EdgeWithNodes {
                        source_node: in_index,
                        target_node: out_index,
                        edge: edge_index,
                    });
                }
                SwapMode::ExactOut => {
                    out_edges_per_mint_index[out_index].push(EdgeWithNodes {
                        source_node: out_index,
                        target_node: in_index,
                        edge: edge_index,
                    });
                }
            }

            has_edge_for_mint.insert(in_key);
            has_edge_for_mint.insert(out_key);
            valid_edge_count += 1;
            skipped_bad_price_impact -= 1;
        }

        if valid_edge_count > 0 {
            warn!(valid_edge_count, skipped_bad_price_impact, "pruning");
        }

        // for mint_vec in out_edges_per_mint_index.iter() {
        //     for mint in mint_vec {
        //         let input_mint = mint_to_index.iter().filter(|(_, x)| **x==mint.source_node).map(|(pk,_)| *pk).collect_vec();
        //         let output_mint = mint_to_index.iter().filter(|(_, x)| **x==mint.target_node).map(|(pk,_)| *pk).collect_vec();
        //         info!("input_mint {:?} {:?}", input_mint, output_mint );
        //     }
        // }

        (valid_edge_count, out_edges_per_mint_index)
    }

    fn update_lowest_price_impact(
        lower_price_impact_edge_for_mint_and_direction: &mut HashMap<
            (MintNodeIndex, bool), // (mint index, is mint 'input' mint for edge)
            (f64, EdgeIndex),
        >,
        edge_index: EdgeIndex,
        key: (MintNodeIndex, bool),
        price_impact: f64,
    ) {
        match lower_price_impact_edge_for_mint_and_direction.entry(key) {
            Occupied(mut e) => {
                if e.get().0 > price_impact {
                    e.insert((price_impact, edge_index.clone()));
                }
            }
            Vacant(e) => {
                e.insert((price_impact, edge_index.clone()));
            }
        }
    }

    // called per request
    #[tracing::instrument(skip_all, level = "trace")]
    fn lookup_edge_index_paths<'a>(
        &self,
        paths: impl Iterator<Item = &'a Vec<EdgeIndex>>,
    ) -> Vec<Vec<Arc<Edge>>> {
        paths
            .map(|path| {
                path.iter()
                    .map(|&edge_index| self.edges[edge_index.idx()].clone())
                    .collect_vec()
            })
            .collect_vec()
    }

    fn edge_info(&self, edge_index: EdgeIndex, _now_ms: u64, in_amount: u64) -> Option<EdgeInfo> {
        let edge = &self.edges[edge_index.idx()];
        let price = edge
            .state
            .read()
            .unwrap()
            .cached_price_for(in_amount)
            .map(|(price, _ln_price)| price)?;

        Some(EdgeInfo {
            price,
            accounts: edge.accounts_needed,
        })
    }

    fn edge_info_exact_out(
        &self,
        edge_index: EdgeIndex,
        _now_ms: u64,
        amount: u64,
    ) -> Option<EdgeInfo> {
        let edge = &self.edges[edge_index.idx()];
        let price = edge
            .state
            .read()
            .unwrap()
            .cached_price_exact_out_for(amount)
            .map(|(price, _ln_price)| price)?;
        Some(EdgeInfo {
            price,
            accounts: edge.accounts_needed,
        })
    }

    pub fn prepare_cache_for_input_mint<F>(
        &self,
        input_mint: &Pubkey,
        in_amount: u64,
        max_accounts: usize,
        filter: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&Pubkey, &Pubkey) -> bool,
    {
        // signer + autobahn-executor program + token program + source token account (others are part of the edges)
        // + ATA program + system program + mint
        let min_accounts_needed = 7;

        let Some(&input_index) = self.mint_to_index.get(input_mint) else {
            bail!("unsupported input mint {input_mint}"); // TODO
        };

        let timestamp = millis_since_epoch();

        let new_paths_by_out_node = self.generate_best_paths(
            in_amount,
            timestamp,
            max_accounts,
            min_accounts_needed,
            input_index,
            &self
                .pruned_out_edges_per_mint_index_exact_in
                .read()
                .unwrap()
                .1,
            &HashSet::new(),
            false,
            self.max_path_length,
        )?;

        let new_paths_by_out_node_exact_out = self.generate_best_paths_exact_out(
            in_amount,
            timestamp,
            max_accounts,
            min_accounts_needed,
            input_index,
            &self
                .pruned_out_edges_per_mint_index_exact_out
                .read()
                .unwrap()
                .1,
            &HashSet::new(),
            false,
            self.max_path_length,
        )?;

        let mut writer = self.path_discovery_cache.write().unwrap();
        for (out_index, new_paths) in new_paths_by_out_node.into_iter().enumerate() {
            let out_index: MintNodeIndex = out_index.into();
            if !filter(&input_mint, &self.mints[out_index]) {
                continue;
            }
            writer.insert(
                input_index,
                out_index,
                SwapMode::ExactIn,
                in_amount,
                max_accounts,
                timestamp,
                new_paths,
            );
        }

        for (out_index, new_paths) in new_paths_by_out_node_exact_out.into_iter().enumerate() {
            let out_index: MintNodeIndex = out_index.into();
            if !filter(&input_mint, &self.mints[out_index]) {
                continue;
            }
            writer.insert(
                input_index,
                out_index,
                SwapMode::ExactOut,
                in_amount,
                max_accounts,
                timestamp,
                new_paths,
            );
        }

        Ok(())
    }

    fn prepare(
        s: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
        e: &Arc<Edge>,
        c: &AccountProviderView,
    ) -> Option<Arc<dyn DexEdge>> {
        s.entry(e.unique_id())
            .or_insert_with(move || e.prepare(c).ok())
            .clone()
    }

    fn compute_out_amount_from_path(
        chain_data: &AccountProviderView,
        snap: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
        path: &[Arc<Edge>],
        amount: u64,
        add_cooldown: bool,
    ) -> anyhow::Result<Option<(u64, u64)>> /* (quote price, cached price) */ {
        let mut current_in_amount = amount;
        let mut current_in_amount_dumb = amount;
        let prepare = Self::prepare;

        for edge in path {
            if !edge.state.read().unwrap().is_valid() {
                warn!(edge = edge.desc(), "invalid edge");
                return Ok(None);
            }
            let prepared_quote = match prepare(snap, edge, chain_data) {
                Some(p) => p,
                _ => bail!(RoutingError::CouldNotComputeOut),
            };
            let quote_res = edge.quote(&prepared_quote, chain_data, current_in_amount);
            let Ok(quote) = quote_res else {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(
                    edge = edge.desc(),
                    amount,
                    "failed to quote, err: {:?}",
                    quote_res.unwrap_err()
                );
                return Ok(None);
            };

            if quote.out_amount == 0 {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(edge = edge.desc(), amount, "quote is zero, skipping");
                return Ok(None);
            }

            let Some(price) = edge
                .state
                .read()
                .unwrap()
                .cached_price_for(current_in_amount)
            else {
                return Ok(None);
            };

            current_in_amount = quote.out_amount;
            current_in_amount_dumb = ((quote.in_amount as f64) * price.0).round() as u64;

            if current_in_amount_dumb > current_in_amount.saturating_mul(3) {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(
                    out_quote = quote.out_amount,
                    out_dumb = current_in_amount_dumb,
                    in_quote = quote.in_amount,
                    price = price.0,
                    edge = edge.desc(),
                    input_mint = debug_tools::name(&edge.input_mint),
                    output_mint = debug_tools::name(&edge.output_mint),
                    prices = edge
                        .state
                        .read()
                        .unwrap()
                        .cached_prices
                        .iter()
                        .map(|x| format!("in={}, price={}", x.0, x.1))
                        .join("||"),
                    "recomputed path amount diverge a lot from estimation - path ignored"
                );
                return Ok(None);
            }
        }
        Ok(Some((current_in_amount, current_in_amount_dumb)))
    }

    fn compute_in_amount_from_path(
        chain_data: &AccountProviderView,
        snap: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
        path: &[Arc<Edge>],
        amount: u64,
        add_cooldown: bool,
    ) -> anyhow::Result<Option<(u64, u64)>> /* (quote price, cached price) */ {
        let prepare = Self::prepare;

        let mut current_out_amount = amount;
        let mut current_out_amount_dumb = amount;
        for edge in path {
            if !edge.supports_exact_out() {
                return Ok(None);
            }

            if !edge.state.read().unwrap().is_valid() {
                warn!(edge = edge.desc(), "invalid edge");
                return Ok(None);
            }
            let prepared_quote = match prepare(snap, edge, chain_data) {
                Some(p) => p,
                _ => bail!(RoutingError::CouldNotComputeOut),
            };
            let quote_res = edge.quote_exact_out(&prepared_quote, chain_data, current_out_amount);
            let Ok(quote) = quote_res else {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(
                    edge = edge.desc(),
                    amount,
                    "failed to quote, err: {:?}",
                    quote_res.unwrap_err()
                );
                return Ok(None);
            };

            if quote.out_amount == 0 {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(edge = edge.desc(), amount, "quote is zero, skipping");
                return Ok(None);
            }

            let Some(price) = edge
                .state
                .read()
                .unwrap()
                .cached_price_exact_out_for(amount)
            else {
                return Ok(None);
            };

            current_out_amount = quote.in_amount;
            current_out_amount_dumb = ((quote.out_amount as f64) * price.0).round() as u64;

            if current_out_amount_dumb > current_out_amount.saturating_mul(3) {
                if add_cooldown {
                    edge.state
                        .write()
                        .unwrap()
                        .add_cooldown(&Duration::from_secs(30));
                }
                warn!(
                    out_quote = quote.out_amount,
                    out_dumb = current_out_amount_dumb,
                    in_quote = quote.in_amount,
                    price = price.0,
                    edge = edge.desc(),
                    input_mint = debug_tools::name(&edge.input_mint),
                    output_mint = debug_tools::name(&edge.output_mint),
                    prices = edge
                        .state
                        .read()
                        .unwrap()
                        .cached_prices
                        .iter()
                        .map(|x| format!("in={}, price={}", x.0, x.1))
                        .join("||"),
                    "recomputed path amount diverge a lot from estimation - path ignored"
                );
                return Ok(None);
            }
        }
        Ok(Some((current_out_amount, current_out_amount_dumb)))
    }

    pub fn find_best_route(
        &self,
        chain_data: &AccountProviderView,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        original_amount: u64,
        max_accounts: usize,
        ignore_cache: bool,
        hot_mints: &HashSet<Pubkey>,
        max_path_length: Option<usize>,
        swap_mode: SwapMode,
    ) -> anyhow::Result<Route> {
        self.prepare_pruned_edges_if_not_initialized(hot_mints, swap_mode);

        // First try with one less hop that maximal authorized as it's way quicker (20-30%)
        // If we can't find anything, will retry with +1
        let max_path_length = max_path_length.unwrap_or((self.max_path_length - 1).max(1));

        // overestimate requested `in_amount` so that quote is not too tight for exact in,
        // exact out we do not overquote here but we will overquote when route is built
        let amount = (original_amount as f64 * (1.0 + self.overquote)).round() as u64;

        // signer + autobahn-executor program + token program + source token account (others are part of the edges)
        // + ATA program + system program + mint
        let min_accounts_needed = 7;

        // Multiple steps:
        // 1. Path discovery: which paths are plausibly good? (expensive, should be cached)
        // 2. Path evaluation/optimization: do actual path quoting, try to find a multi-path route
        // 3. Output generation

        let Some(&input_index) = self.mint_to_index.get(input_mint) else {
            bail!(RoutingError::UnsupportedInputMint(input_mint.clone()));
        };
        let Some(&output_index) = self.mint_to_index.get(output_mint) else {
            bail!(RoutingError::UnsupportedOutputMint(output_mint.clone()));
        };

        trace!(
            input_index = input_index.idx_raw(),
            output_index = output_index.idx_raw(),
            max_path_length,
            "find_best_route"
        );

        // Path discovery: find candidate paths for the pair.
        // Prefer cached paths where possible.
        let cached_paths_opt = {
            let mut cache = self.path_discovery_cache.write().unwrap();
            let cached = cache.get(input_index, output_index, swap_mode, amount, max_accounts);

            let p1 = cached
                .0
                .map(|paths| self.lookup_edge_index_paths(paths.iter()));
            let p2 = cached
                .1
                .map(|paths| self.lookup_edge_index_paths(paths.iter()));

            if (p1.is_none() && p2.is_none()) || ignore_cache {
                None
            } else {
                let cached_paths = p1
                    .unwrap_or(vec![])
                    .into_iter()
                    .chain(p2.unwrap_or(vec![]).into_iter())
                    .collect_vec();
                Some(cached_paths)
            }
        };

        let timestamp = millis_since_epoch();
        let pruned = match swap_mode {
            SwapMode::ExactIn => self
                .pruned_out_edges_per_mint_index_exact_in
                .read()
                .unwrap(),
            SwapMode::ExactOut => self
                .pruned_out_edges_per_mint_index_exact_out
                .read()
                .unwrap(),
        };
        let out_edges_per_node = &pruned.1;

        let mut paths;
        let mut used_cached_paths = false;
        if let Some(cached_paths) = cached_paths_opt {
            paths = cached_paths;
            used_cached_paths = true;
        } else {
            let avoid_cold_mints = !ignore_cache;
            let hot_mints = hot_mints
                .iter()
                .filter_map(|x| self.mint_to_index.get(x))
                .copied()
                .collect();

            let (out_paths, new_paths_by_out_node) = match swap_mode {
                SwapMode::ExactIn => {
                    let new_paths_by_out_node = self.generate_best_paths(
                        amount,
                        timestamp,
                        max_accounts,
                        min_accounts_needed,
                        input_index,
                        &out_edges_per_node,
                        &hot_mints,
                        avoid_cold_mints,
                        max_path_length,
                    )?;
                    (
                        self.lookup_edge_index_paths(new_paths_by_out_node[output_index].iter()),
                        new_paths_by_out_node,
                    )
                }
                SwapMode::ExactOut => {
                    let new_paths_by_out_node = self.generate_best_paths_exact_out(
                        amount,
                        timestamp,
                        max_accounts,
                        min_accounts_needed,
                        output_index,
                        &out_edges_per_node,
                        &hot_mints,
                        avoid_cold_mints,
                        max_path_length,
                    )?;
                    (
                        self.lookup_edge_index_paths(new_paths_by_out_node[input_index].iter()),
                        new_paths_by_out_node,
                    )
                }
            };
            paths = out_paths;

            for (out_index, new_paths) in new_paths_by_out_node.into_iter().enumerate() {
                let out_index: MintNodeIndex = out_index.into();
                self.path_discovery_cache.write().unwrap().insert(
                    input_index,
                    out_index,
                    swap_mode,
                    amount,
                    max_accounts,
                    millis_since_epoch(),
                    new_paths,
                );
            }
        }

        // Path discovery: add all direct paths
        // note: currently this could mean some path exist twice
        self.add_direct_paths(input_index, output_index, out_edges_per_node, &mut paths);

        // Do not keep that locked - deadlock with recursion and also may impact performance
        drop(pruned);

        // Path evaluation
        // TODO: this could evaluate pairs of paths to see if a split makes sense,
        // needs to take care of shared steps though.

        let mut snapshot = HashMap::new();

        let path_output_fn = match swap_mode {
            SwapMode::ExactIn => Self::compute_out_amount_from_path,
            SwapMode::ExactOut => Self::compute_in_amount_from_path,
        };

        let mut path_and_output = paths
            .into_iter()
            .filter_map(|path| {
                path_output_fn(chain_data, &mut snapshot, &path, amount, true)
                    .ok()
                    .flatten()
                    .filter(|v| v.0 > 0)
                    .map(|v| (path, v.0, v.1))
            })
            .collect_vec();

        match swap_mode {
            SwapMode::ExactIn => path_and_output.sort_by_key(|(_, v, _)| std::cmp::Reverse(*v)),
            SwapMode::ExactOut => path_and_output.sort_by_key(|(_, v, _)| *v),
        }

        // Debug
        if tracing::event_enabled!(Level::TRACE) {
            for (path, out_amount, out_amount_dumb) in &path_and_output {
                trace!(
                    "potential path: [out={}] [dumb={}] {}",
                    out_amount,
                    out_amount_dumb,
                    path.iter().map(|edge| edge.desc()).join(", ")
                );
            }
        }

        // Build the output

        for (out_path, routing_result, _) in path_and_output {
            let (route_steps, context_slot) = match swap_mode {
                // Restore requested `in_amount` for route building here
                SwapMode::ExactIn => Self::build_route_steps(
                    chain_data,
                    &mut snapshot,
                    Self::prepare,
                    &out_path,
                    original_amount,
                )?,
                SwapMode::ExactOut => Self::build_route_steps_exact_out(
                    chain_data,
                    &mut snapshot,
                    Self::prepare,
                    &out_path,
                    original_amount,
                )?,
            };

            let actual_in_amount = route_steps.first().unwrap().in_amount;
            let actual_out_amount = route_steps.last().unwrap().out_amount;

            let overquote_in_amount = match swap_mode {
                SwapMode::ExactIn => amount,
                SwapMode::ExactOut => routing_result,
            };

            let out_amount_for_small_amount = Self::compute_out_amount_from_path(
                chain_data,
                &mut snapshot,
                &out_path,
                1_000,
                false,
            )?
            .unwrap_or_default()
            .0;

            let out_amount_for_request = actual_out_amount;
            let expected_ratio = 1_000.0 / out_amount_for_small_amount as f64;
            let actual_ratio = actual_in_amount as f64 / actual_out_amount as f64;

            let price_impact = expected_ratio / actual_ratio * 10_000.0 - 10_000.0;
            let price_impact_bps = price_impact.round() as u64;

            trace!(
                price_impact_bps,
                out_amount_for_small_amount,
                out_amount_for_request,
                expected_ratio,
                actual_ratio,
                "price impact"
            );

            let adjusted_out_amount = match swap_mode {
                SwapMode::ExactIn => ((actual_in_amount as f64 / overquote_in_amount as f64)
                    * routing_result as f64)
                    .floor() as u64,
                SwapMode::ExactOut => original_amount,
            };

            let adjusted_in_amount = match swap_mode {
                SwapMode::ExactIn => actual_in_amount,
                SwapMode::ExactOut => ((actual_out_amount as f64 / amount as f64)
                    * routing_result as f64)
                    .ceil() as u64,
            };

            let out_amount_for_overquoted_amount = match swap_mode {
                SwapMode::ExactIn => routing_result,
                SwapMode::ExactOut => amount,
            };

            if (swap_mode == SwapMode::ExactOut
                && (actual_in_amount == u64::MAX || actual_out_amount == 0))
                || (swap_mode == SwapMode::ExactIn && adjusted_out_amount == 0)
            {
                continue;
            }

            if self.overquote > 0.0 {
                debug!(
                    actual_in_amount,
                    actual_out_amount,
                    overquote_in_amount,
                    out_amount_for_overquoted_amount,
                    adjusted_out_amount,
                    "adjusted amount"
                );
            }

            // If enabled,for debug purpose, recompute route while capturing accessed chain accounts
            // Can be used when executing the swap to check if accounts have changed
            let accounts = self
                .capture_accounts(chain_data, &out_path, original_amount)
                .ok();

            return Ok(Route {
                input_mint: *input_mint,
                output_mint: *output_mint,
                in_amount: adjusted_in_amount,
                out_amount: adjusted_out_amount,
                steps: route_steps,
                slot: context_slot,
                price_impact_bps,
                accounts,
            });
        }

        // No acceptable path
        self.find_route_with_relaxed_constraints_or_fail(
            &chain_data,
            input_mint,
            swap_mode,
            output_mint,
            amount,
            max_accounts,
            ignore_cache,
            hot_mints,
            max_path_length,
            input_index,
            output_index,
            used_cached_paths,
        )
    }

    fn find_route_with_relaxed_constraints_or_fail(
        &self,
        chain_data: &AccountProviderView,
        input_mint: &Pubkey,
        swap_mode: SwapMode,
        output_mint: &Pubkey,
        amount: u64,
        max_accounts: usize,
        ignore_cache: bool,
        hot_mints: &HashSet<Pubkey>,
        max_path_length: usize,
        input_index: MintNodeIndex,
        output_index: MintNodeIndex,
        used_cached_paths: bool,
    ) -> anyhow::Result<Route> {
        // It is possible for cache path to became invalid after some account write or failed tx (cooldown)
        // If we used cache but can't find any valid path, try again without the cache
        let can_try_one_more_hop = max_path_length != self.max_path_length;
        if !ignore_cache && (used_cached_paths || can_try_one_more_hop) {
            if used_cached_paths {
                debug!("Invalid cached path, retrying without cache");
                let mut cache = self.path_discovery_cache.write().unwrap();
                cache.invalidate(input_index, output_index, max_accounts);
            } else {
                debug!("No path within boundaries, retrying with +1 hop");
            }
            return self.find_best_route(
                chain_data,
                input_mint,
                output_mint,
                amount,
                max_accounts,
                used_cached_paths,
                hot_mints,
                Some(self.max_path_length),
                swap_mode,
            );
        }

        // self.print_debug_data(input_mint, output_mint, max_accounts);

        bail!(RoutingError::NoPathBetweenMintPair(
            input_mint.clone(),
            output_mint.clone()
        ));
    }

    fn capture_accounts(
        &self,
        chain_data: &AccountProviderView,
        out_path: &Vec<Arc<Edge>>,
        original_in_amount: u64,
    ) -> anyhow::Result<HashMap<Pubkey, AccountData>> {
        #[cfg(not(feature = "capture-accounts"))]
        return Ok(Default::default());

        #[cfg(feature = "capture-accounts")]
        {
            let mut snapshot = HashMap::new();
            let prepare = |s: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
                           e: &Arc<Edge>,
                           c: &AccountProviderView|
             -> Option<Arc<dyn DexEdge>> {
                s.entry(e.unique_id())
                    .or_insert_with(move || e.prepare(c).ok())
                    .clone()
            };

            let chain_data = Arc::new(ChainDataCaptureAccountProvider::new(chain_data.clone()));
            let downcasted = chain_data.clone() as AccountProviderView;

            Self::build_route_steps(
                &downcasted,
                &mut snapshot,
                prepare,
                &out_path,
                original_in_amount,
            )?;

            let accounts = chain_data.accounts.read().unwrap().clone();
            for (acc, _) in &accounts {
                if !debug_tools::is_in_global_filters(acc) {
                    error!(
                        "Used an account that we are not listening to, addr={:?} !",
                        acc
                    )
                }
            }

            Ok(accounts)
        }
    }

    // per request then per edge down the path
    #[tracing::instrument(skip_all, level = "trace")]
    fn build_route_steps(
        chain_data: &AccountProviderView,
        mut snapshot: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
        prepare: fn(
            &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
            &Arc<Edge>,
            &AccountProviderView,
        ) -> Option<Arc<dyn DexEdge>>,
        out_path: &Vec<Arc<Edge>>,
        in_amount: u64,
    ) -> anyhow::Result<(Vec<RouteStep>, u64)> {
        let mut context_slot = 0;
        let mut steps = Vec::with_capacity(out_path.len());
        let mut current_in_amount = in_amount;
        for edge in out_path.iter() {
            let prepared_quote = match prepare(&mut snapshot, edge, chain_data) {
                Some(p) => p,
                _ => bail!(RoutingError::CouldNotComputeOut),
            };

            let quote = edge.quote(&prepared_quote, chain_data, current_in_amount)?;
            steps.push(RouteStep {
                edge: edge.clone(),
                in_amount: quote.in_amount,
                out_amount: quote.out_amount,
                fee_amount: quote.fee_amount,
                fee_mint: quote.fee_mint,
            });
            current_in_amount = quote.out_amount;
            let edge_slot = edge.state.read().unwrap().last_update_slot;
            context_slot = edge_slot.max(context_slot);
        }

        Ok((steps, context_slot))
    }

    #[tracing::instrument(skip_all, level = "trace")]
    fn build_route_steps_exact_out(
        chain_data: &AccountProviderView,
        mut snapshot: &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
        prepare: fn(
            &mut HashMap<(Pubkey, Pubkey), Option<Arc<dyn DexEdge>>>,
            &Arc<Edge>,
            &AccountProviderView,
        ) -> Option<Arc<dyn DexEdge>>,
        out_path: &Vec<Arc<Edge>>,
        out_amount: u64,
    ) -> anyhow::Result<(Vec<RouteStep>, u64)> {
        let mut context_slot = 0;
        let mut steps = Vec::with_capacity(out_path.len());
        let mut current_out_amount = out_amount;
        for edge in out_path.iter() {
            let prepared_quote = match prepare(&mut snapshot, edge, chain_data) {
                Some(p) => p,
                _ => bail!(RoutingError::CouldNotComputeOut),
            };

            let quote = edge.quote_exact_out(&prepared_quote, chain_data, current_out_amount)?;
            steps.push(RouteStep {
                edge: edge.clone(),
                in_amount: quote.in_amount,
                out_amount: quote.out_amount,
                fee_amount: quote.fee_amount,
                fee_mint: quote.fee_mint,
            });
            current_out_amount = quote.in_amount;
            let edge_slot = edge.state.read().unwrap().last_update_slot;
            context_slot = edge_slot.max(context_slot);
        }

        // reverse the steps for exact out
        steps.reverse();

        Ok((steps, context_slot))
    }

    fn add_direct_paths(
        &self,
        input_index: MintNodeIndex,
        output_index: MintNodeIndex,
        out_edges_per_node: &MintVec<Vec<EdgeWithNodes>>,
        paths: &mut Vec<Vec<Arc<Edge>>>,
    ) {
        for target_and_edge in &out_edges_per_node[input_index] {
            if target_and_edge.target_node != output_index {
                continue;
            }
            let edge = &self.edges[target_and_edge.edge.idx()];
            let state = edge.state.read().unwrap();
            if !state.is_valid() {
                continue;
            }

            if !paths
                .iter()
                .any(|x| x.len() == 1 && x[0].unique_id() == edge.unique_id())
            {
                paths.push(vec![edge.clone()]);
            }
        }
    }

    // called once per request
    #[tracing::instrument(skip_all, level = "trace")]
    fn generate_best_paths(
        &self,
        in_amount: u64,
        now_ms: u64,
        max_accounts: usize,
        min_accounts_needed: usize,
        input_index: MintNodeIndex,
        out_edges_per_node: &MintVec<Vec<EdgeWithNodes>>,
        hot_mints: &HashSet<MintNodeIndex>,
        avoid_cold_mints: bool,
        max_path_length: usize,
    ) -> anyhow::Result<MintVec<Vec<Vec<EdgeIndex>>>> {
        // non-pooled version
        // let mut best_by_node_prealloc = vec![vec![0f64; 3]; 8 * out_edges_per_node.len()];
        let mut best_by_node_prealloc = self.objectpools.get_best_by_node(out_edges_per_node.len());

        // non-pooled version
        // let mut best_paths_by_node_prealloc: MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>> =
        //     MintVec::new_from_prototype(
        //         out_edges_per_node.len(),
        //         vec![(NotNan::new(f64::NEG_INFINITY).unwrap(), vec![]); self.retain_path_count],
        //     );
        let mut best_paths_by_node_prealloc = self
            .objectpools
            .get_best_paths_by_node(out_edges_per_node.len(), self.retain_path_count);

        let new_paths_by_out_node = best_price_paths_depth_search(
            input_index,
            in_amount,
            max_path_length,
            max_accounts.saturating_sub(min_accounts_needed),
            out_edges_per_node,
            &mut best_paths_by_node_prealloc,
            &mut best_by_node_prealloc,
            |edge_index, in_amount| self.edge_info(edge_index, now_ms, in_amount),
            hot_mints,
            avoid_cold_mints,
            SwapMode::ExactIn,
        )?;

        let mut best_paths_by_out_node =
            MintVec::new_from_prototype(new_paths_by_out_node.len(), vec![]);
        // len is some 400000
        trace!("new_paths_by_out_node len={}", new_paths_by_out_node.len());

        for (out_node, best_paths) in new_paths_by_out_node.into_iter().enumerate() {
            let out_node: MintNodeIndex = out_node.into();
            let mut paths = Vec::with_capacity(best_paths.len());

            for (_, path) in best_paths {
                let edges = path.into_iter().map(|edge| edge.edge).collect();
                paths.push(edges);
            }

            best_paths_by_out_node[out_node] = paths;
        }

        Ok(best_paths_by_out_node)
    }

    // called once per request
    #[tracing::instrument(skip_all, level = "trace")]
    fn generate_best_paths_exact_out(
        &self,
        out_amount: u64,
        now_ms: u64,
        max_accounts: usize,
        min_accounts_needed: usize,
        output_index: MintNodeIndex,
        out_edges_per_node: &MintVec<Vec<EdgeWithNodes>>,
        hot_mints: &HashSet<MintNodeIndex>,
        avoid_cold_mints: bool,
        max_path_length: usize,
    ) -> anyhow::Result<MintVec<Vec<Vec<EdgeIndex>>>> {
        // similar to generate_best_paths justg changing function to calculate edge info and setting is_exact_out to true
        let mut best_by_node_prealloc = self.objectpools.get_best_by_node(out_edges_per_node.len());

        let mut best_paths_by_node_prealloc = self
            .objectpools
            .get_best_paths_by_node_exact_out(out_edges_per_node.len(), self.retain_path_count);

        let new_paths_by_out_node = best_price_paths_depth_search(
            output_index,
            out_amount,
            max_path_length,
            max_accounts.saturating_sub(min_accounts_needed),
            out_edges_per_node,
            &mut best_paths_by_node_prealloc,
            &mut best_by_node_prealloc,
            |edge_index, out_amount: u64| self.edge_info_exact_out(edge_index, now_ms, out_amount),
            hot_mints,
            avoid_cold_mints,
            SwapMode::ExactOut,
        )?;

        let mut best_paths_by_out_node =
            MintVec::new_from_prototype(new_paths_by_out_node.len(), vec![]);
        trace!("new_paths_by_out_node len={}", new_paths_by_out_node.len());

        for (out_node, best_paths) in new_paths_by_out_node.into_iter().enumerate() {
            let out_node: MintNodeIndex = out_node.into();
            let mut paths = Vec::with_capacity(best_paths.len());

            for (_, path) in best_paths {
                let edges = path.into_iter().map(|edge| edge.edge).collect();
                paths.push(edges);
            }

            best_paths_by_out_node[out_node] = paths;
        }

        Ok(best_paths_by_out_node)
    }

    pub fn find_edge(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        amm_key: Pubkey,
    ) -> anyhow::Result<Arc<Edge>> {
        if let Some(result) = self.edges.iter().find(|x| {
            x.input_mint == input_mint && x.output_mint == output_mint && x.key() == amm_key
        }) {
            Ok(result.clone())
        } else {
            Err(anyhow::format_err!("Edge not found"))
        }
    }

    fn print_debug_data(&self, input_mint: &Pubkey, output_mint: &Pubkey, max_accounts: usize) {
        warn!(
            %input_mint,
            %output_mint, max_accounts, "Couldn't find a path"
        );

        let mut seen_out = HashSet::new();
        let mut seen_in = HashSet::new();
        for edge in &self.edges {
            if edge.output_mint == *output_mint {
                Self::print_some_edges(&mut seen_out, edge);
            }
            if edge.input_mint == *input_mint {
                Self::print_some_edges(&mut seen_in, edge);
            }
        }
    }

    fn print_some_edges(seen: &mut HashSet<(Pubkey, Pubkey)>, edge: &Arc<Edge>) {
        if seen.insert(edge.unique_id()) == false || seen.len() > 6 {
            return;
        }
        let (valid, prices) = {
            let reader = edge.state.read().unwrap();
            let prices = reader
                .cached_prices
                .iter()
                .map(|x| format!("q={} @ p={}", x.0, x.1))
                .join(" // ");
            (reader.is_valid(), prices)
        };
        warn!(
            edge = edge.id.desc(),
            input = debug_tools::name(&edge.input_mint),
            output = debug_tools::name(&edge.output_mint),
            valid,
            prices,
            " - available edge"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::test::MockDexIdentifier;
    use crate::mock::test::MockDexInterface;
    use crate::routing_objectpool::{
        alloc_best_by_node_for_test, alloc_best_paths_by_node_for_test,
    };
    use router_lib::chain_data::ChainDataArcRw;
    use router_lib::dex::{ChainDataAccountProvider, DexInterface};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn make_graph(
        edges: &[(&str, &str, f64)],
    ) -> (
        HashMap<String, MintNodeIndex>,
        MintVec<Vec<EdgeWithNodes>>,
        Vec<f64>,
    ) {
        let edge_prices = edges.iter().map(|(_, _, p)| *p).collect_vec();

        let mut nodes = HashMap::<String, MintNodeIndex>::new();
        let mut add_node = |name: &str| {
            if !nodes.contains_key(name) {
                nodes.insert(name.to_string(), nodes.len().into());
            }
        };
        for (from, to, _) in edges {
            add_node(from);
            add_node(to);
        }

        let mut out_edges_per_node = MintVec::new_from_prototype(nodes.len(), vec![]);
        for (edge_index, (from, to, _)) in edges.iter().enumerate() {
            let edge_index = edge_index.into();
            let from_index = nodes.get(&from.to_string()).unwrap();
            let to_index = nodes.get(&to.to_string()).unwrap();
            out_edges_per_node[*from_index].push(EdgeWithNodes {
                edge: edge_index,
                source_node: *from_index,
                target_node: *to_index,
            });
        }

        (nodes, out_edges_per_node, edge_prices)
    }

    macro_rules! assert_eq_f64 {
        ($actual:expr, $expected:expr, $delta:expr) => {
            if ($actual - $expected).abs() > $delta {
                println!(
                    "assertion failed: {} is not approximately {}",
                    $actual, $expected
                );
                assert!(false);
            }
        };
    }

    macro_rules! check_path {
        ($path:expr, $expected_cost:expr, $expected_nodes:expr, $node_lookup:expr) => {
            let path = $path;
            let node_lookup = $node_lookup;
            let path_nodes = path.1.iter().map(|edge| edge.target_node).collect_vec();
            let expected_path_nodes = $expected_nodes
                .iter()
                .map(|node| *node_lookup.get(&node.to_string()).unwrap())
                .collect_vec();
            assert_eq!(path_nodes, expected_path_nodes);
            assert_eq_f64!(path.0, $expected_cost, 0.000001);
        };
    }

    #[test]
    fn find_best_paths() {
        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 0.99),
            ("USDC", "USDT", 0.95),
            ("USDT", "USDC", 2.0), // never taken when starting from USDC: no cycles allowed
            ("USDC", "SOL", 0.99 / 200.0),
            ("SOL", "USDT", 0.98 * 200.0),
            ("SOL", "DAI", 1.05 * 200.0),
            ("DAI", "USDT", 0.97),
        ]);
        let edge_info_fn = |edge: EdgeIndex, _in_amount| {
            Some(EdgeInfo {
                price: edge_prices[edge.idx()],
                accounts: 0,
            })
        };
        let get_paths = |from: &str, to: &str, max_length| {
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                1,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    5,
                    SwapMode::ExactIn,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_info_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactIn,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        {
            let paths = get_paths("USDC", "USDT", 1);
            check_path!(&paths[0], 0.99, &["USDT"], &nodes);
            check_path!(&paths[1], 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 2);
            check_path!(&paths[0], 0.99, &["USDT"], &nodes);
            check_path!(&paths[1], 0.99 * 0.98, &["SOL", "USDT"], &nodes);
            check_path!(&paths[2], 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 3);
        }
        {
            let paths = get_paths("USDC", "USDT", 3);
            check_path!(
                &paths[0],
                0.99 * 1.05 * 0.97,
                &["SOL", "DAI", "USDT"],
                &nodes
            );
            check_path!(&paths[1], 0.99, &["USDT"], &nodes);
            check_path!(&paths[2], 0.99 * 0.98, &["SOL", "USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
    }

    #[test]
    fn find_best_paths_exact_out() {
        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 0.99),
            ("USDC", "USDT", 0.95),
            ("USDT", "USDC", 1.05), // never taken when starting from USDC: no cycles allowed
            ("USDC", "SOL", 200.0 * 0.98),
            ("SOL", "USDT", 1.0 / 200.0),
            ("SOL", "DAI", 1.0 / 200.0),
            ("DAI", "USDT", 0.95),
        ]);
        let edge_info_fn = |edge: EdgeIndex, _in_amount| {
            Some(EdgeInfo {
                price: edge_prices[edge.idx()],
                accounts: 0,
            })
        };
        let get_paths = |from: &str, to: &str, max_length| {
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                1,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    5,
                    SwapMode::ExactOut,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_info_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactOut,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        let paths = get_paths("USDC", "USDT", 3);
        for (price, edges) in paths {
            println!(
                "{} = {}",
                price,
                edges
                    .iter()
                    .map(|x| format!("({:?}: {:?}->{:?})", x.edge, x.source_node, x.target_node))
                    .join(", ")
            )
        }

        {
            let paths = get_paths("USDC", "USDT", 1);
            check_path!(&paths[0], 0.95, &["USDT"], &nodes);
            check_path!(&paths[1], 0.99, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 2);
            check_path!(&paths[0], 0.95, &["USDT"], &nodes);
            check_path!(&paths[1], 0.98, &["SOL", "USDT"], &nodes);
            check_path!(&paths[2], 0.99, &["USDT"], &nodes);
            assert_eq!(paths.len(), 3);
        }
        {
            let paths = get_paths("USDC", "USDT", 3);
            check_path!(
                &paths[0],
                0.98 * 1.0 * 0.95,
                &["SOL", "DAI", "USDT"],
                &nodes
            );
            check_path!(&paths[1], 0.95, &["USDT"], &nodes);
            check_path!(&paths[2], 0.98, &["SOL", "USDT"], &nodes);
            check_path!(&paths[3], 0.99, &["USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
    }

    /// Check that correct paths are found when the edge prices depend on the input amount
    #[test]
    fn find_best_paths_variable_prices() {
        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 0.99),        // * 0.5
            ("USDC", "USDT", 0.95),        // * 0.8
            ("USDC", "SOL", 0.99 / 200.0), // * 0.8
            ("SOL", "USDT", 0.98 * 200.0), // * 0.8
            ("USDC", "DAI", 1.01),         // * 0.8
            ("DAI", "USDT", 1.02),         // * 0.8
        ]);
        let edge_price_fn = |edge: EdgeIndex, in_amount| {
            let price = if in_amount > 100 {
                edge_prices[edge.idx()] * if edge.idx() == 0 { 0.5 } else { 0.8 }
            } else {
                edge_prices[edge.idx()]
            };
            Some(EdgeInfo { price, accounts: 0 })
        };
        let get_paths = |from: &str, to: &str, in_amount, max_length| {
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                in_amount,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    5,
                    SwapMode::ExactIn,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_price_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactIn,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        {
            let paths = get_paths("USDC", "USDT", 1, 1);
            check_path!(&paths[0], 0.99, &["USDT"], &nodes);
            check_path!(&paths[1], 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 1000, 1);
            check_path!(&paths[0], 1000.0 * 0.95 * 0.8, &["USDT"], &nodes);
            check_path!(&paths[1], 1000.0 * 0.99 * 0.5, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 1, 2);
            check_path!(&paths[0], 1.01 * 1.02, &["DAI", "USDT"], &nodes);
            check_path!(&paths[1], 0.99, &["USDT"], &nodes);
            check_path!(&paths[2], 0.99 * 0.98, &["SOL", "USDT"], &nodes);
            check_path!(&paths[3], 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
        {
            let paths = get_paths("USDC", "USDT", 1000, 2);
            check_path!(
                &paths[0],
                1000.0 * 0.99 * 0.98 * 0.8,
                &["SOL", "USDT"],
                &nodes
            );
            check_path!(&paths[1], 1000.0 * 0.95 * 0.8, &["USDT"], &nodes);
            check_path!(
                &paths[2],
                1000.0 * 1.01 * 1.02 * 0.8 * 0.8,
                &["DAI", "USDT"],
                &nodes
            );
            check_path!(&paths[3], 1000.0 * 0.99 * 0.5, &["USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
    }

    #[test]
    fn find_best_paths_variable_prices_exact_out() {
        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 1.0 / 0.99),          // * 0.5
            ("USDC", "USDT", 1.0 / 0.95),          // * 0.8
            ("USDC", "SOL", 200.0 / 0.99),         // * 0.8
            ("SOL", "USDT", 1.0 / (200.0 * 0.98)), // * 0.8
            ("USDC", "DAI", 1.0 / 1.01),           // * 0.8
            ("DAI", "USDT", 1.0 / 1.02),           // * 0.8
        ]);
        let edge_price_fn = |edge: EdgeIndex, in_amount| {
            let price = if in_amount > 500 {
                edge_prices[edge.idx()] * if edge.idx() == 0 { 0.5 } else { 0.8 }
            } else {
                edge_prices[edge.idx()]
            };
            Some(EdgeInfo { price, accounts: 0 })
        };
        let get_paths = |from: &str, to: &str, in_amount, max_length| {
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                in_amount,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    5,
                    SwapMode::ExactOut,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_price_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactOut,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        {
            let paths = get_paths("USDC", "USDT", 1, 1);
            check_path!(&paths[0], 1.0 / 0.99, &["USDT"], &nodes);
            check_path!(&paths[1], 1.0 / 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 1000, 1);
            check_path!(&paths[0], 1000.0 * 1.0 / 0.99 * 0.5, &["USDT"], &nodes);
            check_path!(&paths[1], 1000.0 * 1.0 / 0.95 * 0.8, &["USDT"], &nodes);
            assert_eq!(paths.len(), 2);
        }
        {
            let paths = get_paths("USDC", "USDT", 1, 2);
            check_path!(&paths[0], 1.0 / 1.01 * 1.0 / 1.02, &["DAI", "USDT"], &nodes);
            check_path!(&paths[1], 1.0 / 0.99, &["USDT"], &nodes);
            check_path!(&paths[2], 1.0 / 0.99 * 1.0 / 0.98, &["SOL", "USDT"], &nodes);
            check_path!(&paths[3], 1.0 / 0.95, &["USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
        {
            let paths = get_paths("USDC", "USDT", 1000, 2);
            check_path!(
                &paths[2],
                1000.0 * 1.0 / 0.99 * 1.0 / 0.98 * 0.8 * 0.8,
                &["SOL", "USDT"],
                &nodes
            );
            check_path!(&paths[3], 1000.0 * 1.0 / 0.95 * 0.8, &["USDT"], &nodes);
            check_path!(
                &paths[1],
                1000.0 * (1.0 / 1.01) * (1.0 / 1.02) * 0.8 * 0.8,
                &["DAI", "USDT"],
                &nodes
            );
            check_path!(&paths[0], 1000.0 * 1.0 / 0.99 * 0.5, &["USDT"], &nodes);
            assert_eq!(paths.len(), 4);
        }
    }

    #[test]
    fn find_best_paths_performance_test() {
        panic_after(Duration::from_secs(3), || {
            let instant = Instant::now();
            let edges: Vec<(&str, &str, f64)> = (1..250)
                .map(|i| {
                    let d = (i as f64) / 10_000f64;
                    vec![
                        ("USDC", "USDT", 0.99 - d),
                        ("USDC", "USDT", 0.95 - d),
                        ("USDT", "USDC", 0.95 - d),
                        ("USDC", "SOL", (0.99 - d) / 200.0),
                        ("INF", "SOL", 1.03 - d),
                        ("SOL", "INF", 0.97 - d),
                        ("SOL", "USDT", (0.98 - d) * 200.0),
                        ("USDC", "TBTC", (0.99 - d) / 60_000.0),
                        ("TBTC", "USDT", (0.98 - d) * 60_000.0),
                        ("USDC", "wETH", (0.99 - d) / 4_000.0),
                        ("wETH", "USDT", (0.98 - d) * 4_000.0),
                        ("wETH", "SOL", (0.988 - d) * 4_000.0 / 200.0),
                        ("SOL", "wETH", (0.983 - d) * 4_000.0 * 200.0),
                        ("USDC", "DAI", 1.01 + d),
                        ("DAI", "USDT", 1.02 + d),
                        ("SOL", "JupSOL", 0.98 - d),
                        ("JupSOL", "USDT", 205.0 + d),
                    ]
                })
                .flatten()
                .collect();

            let (nodes, out_edges_per_node, edge_prices) = make_graph(edges.as_slice());

            let edge_price_fn = |edge: EdgeIndex, in_amount| {
                let price = if in_amount > 100 {
                    edge_prices[edge.idx()] * 0.8
                } else {
                    edge_prices[edge.idx()]
                };
                Some(EdgeInfo { price, accounts: 0 })
            };

            let get_paths = |from: &str, to: &str, in_amount, max_length| {
                best_price_paths_depth_search(
                    *nodes.get(&from.to_string()).unwrap(),
                    in_amount,
                    max_length,
                    64,
                    &out_edges_per_node,
                    &mut alloc_best_paths_by_node_for_test(
                        out_edges_per_node.len(),
                        10,
                        SwapMode::ExactIn,
                    ),
                    &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                    edge_price_fn,
                    &HashSet::new(),
                    false,
                    SwapMode::ExactIn,
                )
                .unwrap()[*nodes.get(&to.to_string()).unwrap()]
                .clone()
            };

            for _i in 0..10 {
                let paths = get_paths("INF", "USDC", 1, 5);
                assert_eq!(paths.is_empty(), false);
            }
            println!("Time taken : {} ms", instant.elapsed().as_millis());
        });
    }

    #[test]
    fn find_best_paths_performance_test_exact_out() {
        panic_after(Duration::from_secs(3), || {
            let instant = Instant::now();
            let edges: Vec<(&str, &str, f64)> = (1..250)
                .map(|i| {
                    let d = (i as f64) / 10_000f64;
                    vec![
                        ("USDC", "USDT", 0.99 - d),
                        ("USDC", "USDT", 0.95 - d),
                        ("USDT", "USDC", 0.95 - d),
                        ("USDC", "SOL", (0.99 - d) / 200.0),
                        ("INF", "SOL", 1.03 - d),
                        ("SOL", "INF", 0.97 - d),
                        ("SOL", "USDT", (0.98 - d) * 200.0),
                        ("USDC", "TBTC", (0.99 - d) / 60_000.0),
                        ("TBTC", "USDT", (0.98 - d) * 60_000.0),
                        ("USDC", "wETH", (0.99 - d) / 4_000.0),
                        ("wETH", "USDT", (0.98 - d) * 4_000.0),
                        ("wETH", "SOL", (0.988 - d) * 4_000.0 / 200.0),
                        ("SOL", "wETH", (0.983 - d) * 4_000.0 * 200.0),
                        ("USDC", "DAI", 1.01 + d),
                        ("DAI", "USDT", 1.02 + d),
                        ("SOL", "JupSOL", 0.98 - d),
                        ("JupSOL", "USDT", 205.0 + d),
                    ]
                })
                .flatten()
                .collect();

            let (nodes, out_edges_per_node, edge_prices) = make_graph(edges.as_slice());

            let edge_price_fn = |edge: EdgeIndex, in_amount| {
                let price = if in_amount > 100 {
                    edge_prices[edge.idx()] * 0.8
                } else {
                    edge_prices[edge.idx()]
                };
                Some(EdgeInfo { price, accounts: 0 })
            };

            let get_paths = |from: &str, to: &str, in_amount, max_length| {
                best_price_paths_depth_search(
                    *nodes.get(&from.to_string()).unwrap(),
                    in_amount,
                    max_length,
                    64,
                    &out_edges_per_node,
                    &mut alloc_best_paths_by_node_for_test(
                        out_edges_per_node.len(),
                        10,
                        SwapMode::ExactOut,
                    ),
                    &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                    edge_price_fn,
                    &HashSet::new(),
                    false,
                    SwapMode::ExactOut,
                )
                .unwrap()[*nodes.get(&to.to_string()).unwrap()]
                .clone()
            };

            for _i in 0..10 {
                let paths = get_paths("INF", "USDC", 1, 5);
                assert_eq!(paths.is_empty(), false);
            }
            println!("Time taken : {} ms", instant.elapsed().as_millis());
        });
    }

    #[test]
    fn should_find_same_top_path_when_asking_for_5_or_for_50_bests() {
        // Doing a USDC->USDT
        // Should find:
        // - Direct A
        // - Direct B
        // - USDC -> SOL -> USDT (D+E)
        // - USDC -> SOL -> DAI -> USDT (D+F+I)
        // - USDC -> BONK -> SOL -> USDT (G+H+E)
        // - USDC -> BONK -> SOL -> DAI -> USDT (G+H+F+I)
        // - USDC -> BONK -> USDT (G+J)
        // - USDC -> BONK -> USDT (G+K)
        // - USDC -> BONK -> SOL -> USDT (L+H+E)
        // - USDC -> BONK -> SOL -> DAI -> USDT (L+H+F+I)
        // - USDC -> BONK -> USDT (L+J)
        // - USDC -> BONK -> USDT (L+K)

        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 0.99),              // A /
            ("USDC", "USDT", 0.95),              // B /
            ("USDT", "USDC", 2.0), // C / never taken when starting from USDC: no cycles allowed
            ("USDC", "SOL", 0.99 / 200.0), // D /
            ("SOL", "USDT", 0.98 * 200.0), // E /
            ("SOL", "DAI", 1.05 * 200.0), // F /
            ("USDC", "BONK", 100_000.0), // G /
            ("BONK", "SOL", 205.0 / 100_000.0), // H /
            ("DAI", "USDT", 0.97), // I /
            ("BONK", "USDT", 1.02 / 100_000.0), // J /
            ("BONK", "USDT", 1.025 / 100_000.0), // K /
            ("USDC", "BONK", 100_005.0), // L /
        ]);
        let edge_info_fn = |edge: EdgeIndex, _in_amount| {
            Some(EdgeInfo {
                price: edge_prices[edge.idx()],
                accounts: 7,
            })
        };

        let get_paths = |from: &str, to: &str, n_path, max_length| {
            // routing.find_best_route()
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                10000,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    n_path,
                    SwapMode::ExactIn,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_info_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactIn,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        let paths1 = get_paths("USDC", "USDT", 5, 5);
        let paths2 = get_paths("USDC", "USDT", 50, 5);

        assert_eq!(paths1.len(), 5);
        assert_eq!(paths2.len(), 12);

        for i in 0..5 {
            assert_eq_f64!(paths1[i].0, paths2[i].0, 0.000001);
        }
    }

    #[test]
    fn should_find_same_top_path_when_asking_for_5_or_for_50_bests_for_exact_out() {
        // Doing a USDC->USDT
        // Should find:
        // - Direct A
        // - Direct B
        // - USDC -> SOL -> USDT (D+E)
        // - USDC -> SOL -> DAI -> USDT (D+F+I)
        // - USDC -> BONK -> SOL -> USDT (G+H+E)
        // - USDC -> BONK -> SOL -> DAI -> USDT (G+H+F+I)
        // - USDC -> BONK -> USDT (G+J)
        // - USDC -> BONK -> USDT (G+K)
        // - USDC -> BONK -> SOL -> USDT (L+H+E)
        // - USDC -> BONK -> SOL -> DAI -> USDT (L+H+F+I)
        // - USDC -> BONK -> USDT (L+J)
        // - USDC -> BONK -> USDT (L+K)

        let (nodes, out_edges_per_node, edge_prices) = make_graph(&[
            ("USDC", "USDT", 0.99),              // A /
            ("USDC", "USDT", 0.95),              // B /
            ("USDT", "USDC", 2.0), // C / never taken when starting from USDC: no cycles allowed
            ("USDC", "SOL", 0.99 / 200.0), // D /
            ("SOL", "USDT", 0.98 * 200.0), // E /
            ("SOL", "DAI", 1.05 * 200.0), // F /
            ("USDC", "BONK", 100_000.0), // G /
            ("BONK", "SOL", 205.0 / 100_000.0), // H /
            ("DAI", "USDT", 0.97), // I /
            ("BONK", "USDT", 1.02 / 100_000.0), // J /
            ("BONK", "USDT", 1.025 / 100_000.0), // K /
            ("USDC", "BONK", 100_005.0), // L /
        ]);
        let edge_info_fn = |edge: EdgeIndex, _in_amount| {
            Some(EdgeInfo {
                price: edge_prices[edge.idx()],
                accounts: 7,
            })
        };

        let get_paths = |from: &str, to: &str, n_path, max_length| {
            // routing.find_best_route()
            best_price_paths_depth_search(
                *nodes.get(&from.to_string()).unwrap(),
                10000,
                max_length,
                64,
                &out_edges_per_node,
                &mut alloc_best_paths_by_node_for_test(
                    out_edges_per_node.len(),
                    n_path,
                    SwapMode::ExactOut,
                ),
                &mut alloc_best_by_node_for_test(out_edges_per_node.len()),
                edge_info_fn,
                &HashSet::new(),
                false,
                SwapMode::ExactOut,
            )
            .unwrap()[*nodes.get(&to.to_string()).unwrap()]
            .clone()
        };

        let paths1 = get_paths("USDC", "USDT", 5, 5);
        let paths2 = get_paths("USDC", "USDT", 50, 5);

        assert_eq!(paths1.len(), 5);
        assert_eq!(paths2.len(), 12);

        for i in 0..5 {
            assert_eq_f64!(paths1[i].0, paths2[i].0, 0.000001);
        }
    }

    #[test]
    fn should_find_best_exact_in_route_fully_integrated() {
        let usdc = Pubkey::new_unique();
        let sol = Pubkey::new_unique();
        let mngo = Pubkey::new_unique();
        let pool_1 = Pubkey::new_unique();
        let pool_2 = Pubkey::new_unique();
        let pool_3 = Pubkey::new_unique();

        //
        let chain_data = Arc::new(ChainDataAccountProvider::new(ChainDataArcRw::new(
            Default::default(),
        ))) as AccountProviderView;
        let dex = Arc::new(MockDexInterface {}) as Arc<dyn DexInterface>;
        let edges = vec![
            Arc::new(make_edge(
                &dex,
                &pool_1,
                &usdc,
                &sol,
                &chain_data,
                6,
                1.0,
                1.0 / 0.1495,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_1,
                &sol,
                &usdc,
                &chain_data,
                9,
                150.0,
                0.1497,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_2,
                &usdc,
                &sol,
                &chain_data,
                6,
                1.0,
                1.0 / 0.1498,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_2,
                &sol,
                &usdc,
                &chain_data,
                9,
                150.0,
                0.1501,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_3,
                &usdc,
                &mngo,
                &chain_data,
                6,
                1.00,
                1.0 / 0.0198,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_3,
                &mngo,
                &usdc,
                &chain_data,
                6,
                0.02,
                0.0197,
            )),
        ];
        let pwa = vec![100, 1000];
        let config = Config {
            ..Config::default()
        };

        let routing = Routing::new(&config, pwa, edges);

        let path = routing
            .find_best_route(
                &chain_data,
                &sol,
                &mngo,
                1_000_000_000,
                40,
                true,
                &Default::default(),
                None,
                SwapMode::ExactIn,
            )
            .unwrap();

        assert_eq!(2, path.steps.len());
        assert_eq!(pool_2, path.steps[0].edge.id.key());
        assert_eq!(pool_3, path.steps[1].edge.id.key());
        assert_eq!(7580808080, path.out_amount);
        assert_eq_f64!(
            1_000_000_000.0 * 0.1501 * 1.0 / 0.0198,
            path.out_amount as f64,
            1.0
        );
    }

    #[test]
    fn should_find_best_exact_in_route_fully_integrated_exact_out() {
        let usdc = Pubkey::new_unique();
        let sol = Pubkey::new_unique();
        let mngo = Pubkey::new_unique();
        let pool_1 = Pubkey::new_unique();
        let pool_2 = Pubkey::new_unique();
        let pool_3 = Pubkey::new_unique();

        //
        let chain_data = Arc::new(ChainDataAccountProvider::new(ChainDataArcRw::new(
            Default::default(),
        ))) as AccountProviderView;
        let dex = Arc::new(MockDexInterface {}) as Arc<dyn DexInterface>;
        let edges = vec![
            Arc::new(make_edge(
                &dex,
                &pool_1,
                &usdc,
                &sol,
                &chain_data,
                6,
                1.0,
                1.0 / 0.1495,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_1,
                &sol,
                &usdc,
                &chain_data,
                9,
                150.0,
                0.1497,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_2,
                &usdc,
                &sol,
                &chain_data,
                6,
                1.0,
                1.0 / 0.1498,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_2,
                &sol,
                &usdc,
                &chain_data,
                9,
                150.0,
                0.1501,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_3,
                &usdc,
                &mngo,
                &chain_data,
                6,
                1.00,
                1.0 / 0.0198,
            )),
            Arc::new(make_edge(
                &dex,
                &pool_3,
                &mngo,
                &usdc,
                &chain_data,
                6,
                0.02,
                0.0197,
            )),
        ];
        let pwa = vec![100, 1000];
        let config = Config {
            ..Config::default()
        };

        let routing = Routing::new(&config, pwa, edges);

        let path = routing
            .find_best_route(
                &chain_data,
                &sol,
                &mngo,
                1_000_000_000,
                40,
                true,
                &Default::default(),
                None,
                SwapMode::ExactOut,
            )
            .unwrap();

        assert_eq!(2, path.steps.len());
        assert_eq!(pool_2, path.steps[0].edge.id.key());
        assert_eq!(pool_3, path.steps[1].edge.id.key());
        assert_eq!(131_912_059, path.in_amount);
        assert_eq!(1_000_000_000, path.out_amount);
        assert_eq_f64!(
            1_000_000_000.0 * 0.0198 * 1.0 / 0.1501,
            path.in_amount as f64,
            1.0
        );
    }

    fn make_edge(
        dex: &Arc<dyn DexInterface>,
        key: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        chain_data: &AccountProviderView,
        decimals: u8,
        input_price_usd: f64,
        pool_price: f64,
    ) -> Edge {
        let edge = Edge {
            input_mint: input_mint.clone(),
            output_mint: output_mint.clone(),
            dex: dex.clone(),
            id: Arc::new(MockDexIdentifier {
                key: key.clone(),
                input_mint: input_mint.clone(),
                output_mint: output_mint.clone(),
                price: pool_price,
            }),
            accounts_needed: 10,
            state: Default::default(),
        };

        edge.update_internal(chain_data, decimals, input_price_usd, &vec![100, 1000]);
        edge
    }

    fn panic_after<T, F>(d: Duration, f: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T,
        F: Send + 'static,
    {
        let (done_tx, done_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let val = f();
            done_tx.send(()).expect("Unable to send completion signal");
            val
        });

        match done_rx.recv_timeout(d) {
            Ok(_) => handle.join().expect("Thread panicked"),
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("Thread took too long"),
            Err(_) => panic!("Something went wrong"),
        }
    }
}
