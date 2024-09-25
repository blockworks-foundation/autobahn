use prometheus::core::GenericGauge;
use prometheus::{
    histogram_opts, opts, register_gauge_vec, register_histogram_vec, register_int_counter,
    register_int_counter_vec, register_int_gauge, register_int_gauge_vec, GaugeVec, HistogramVec,
    IntCounter, IntCounterVec, IntGauge, IntGaugeVec,
};

lazy_static::lazy_static! {
    pub static ref GRPC_ACCOUNT_WRITES: IntCounter =
        register_int_counter!("grpc_account_writes", "Number of account updates via Geyser gRPC").unwrap();
    pub static ref GRPC_ACCOUNT_WRITE_QUEUE: IntGauge =
        register_int_gauge!("grpc_account_write_queue", "Items in account write queue via Geyser gPRC").unwrap();
    pub static ref GRPC_DEDUP_QUEUE: GenericGauge<prometheus::core::AtomicI64> =
        register_int_gauge!("grpc_dedup_queue", "Items in dedup queue via Geyser gPRC").unwrap();
    pub static ref GRPC_SLOT_UPDATE_QUEUE: GenericGauge<prometheus::core::AtomicI64> =
        register_int_gauge!("grpc_slot_update_queue", "Items in slot update queue via Geyser gPRC").unwrap();
    pub static ref GRPC_SLOT_UPDATES: IntCounter =
        register_int_counter!("grpc_slot_updates", "Number of slot updates via Geyser gPRC").unwrap();
    pub static ref ACCOUNT_SNAPSHOTS: IntCounterVec =
        register_int_counter_vec!(opts!("router_account_snapshots", "Number of account snapshots"), &["snapshot_type"]).unwrap();
    pub static ref GRPC_SNAPSHOT_ACCOUNT_WRITES: IntCounter =
        register_int_counter!("router_snapshot_account_writes", "Number of account writes from snapshot").unwrap();
     pub static ref GRPC_SOURCE_CONNECTION_RETRIES: IntCounterVec =
        register_int_counter_vec!(opts!("grpc_source_connection_retries", "gRPC source connection retries"), &["source_name"]).unwrap();
    pub static ref GRPC_NO_MESSAGE_FOR_DURATION_MS: IntGauge =
        register_int_gauge!("grpc_no_update_for_duration_ms", "Did not get any message from Geyser gPRC for this duration").unwrap();
    pub static ref GRPC_TO_EDGE_SLOT_LAG: IntGaugeVec =
        register_int_gauge_vec!(opts!("router_grpc_to_edge_slot_lag", "RPC Slot vs last slot used to update edges"), &["dex_name"]).unwrap();

    pub static ref HTTP_REQUEST_TIMING: HistogramVec =
        register_histogram_vec!(
            histogram_opts!("router_http_request_timing", "Endpoint timing in seconds",
                // buckets (in seconds)
                vec![
                    2e-6, 5e-6, 10e-6, 15e-6, 20e-6, 25e-6, 30e-6, 50e-6, 100e-6, 200e-6, 500e-6,
                    2e-3, 5e-3, 10e-3, 25e-3, 50e-3, 100e-3, 200e-3, 500e-3,
                    1.0, 2.0
                ]),
                &["endpoint", "client"]).unwrap();
    pub static ref HTTP_REQUESTS_TOTAL: IntCounterVec =
        register_int_counter_vec!(opts!("router_http_requests_total", "Number of total endpoint requests"), &["endpoint", "client"]).unwrap();
    pub static ref HTTP_REQUESTS_FAILED: IntCounterVec =
        register_int_counter_vec!(opts!("router_http_requests_failed", "Number of failed endpoint requests"), &["endpoint", "client"]).unwrap();

    pub static ref PATH_DISCOVERY_CACHE_HITS: IntCounter =
        register_int_counter!("router_path_discovery_cache_hits", "Cache hits in path discovery").unwrap();
    pub static ref PATH_DISCOVERY_CACHE_MISSES: IntCounter =
        register_int_counter!("router_path_discovery_cache_misses", "Cache misses in path discovery").unwrap();

    pub static ref OBJECTPOOL_BEST_BY_NODE_NEW_ALLOCATIONS: IntCounter =
        register_int_counter!("router_objectpool_best_by_node_allocations", "Number of new allocations in object pool best_by_node").unwrap();
    pub static ref OBJECTPOOL_BEST_BY_NODE_REUSES: IntCounter =
        register_int_counter!("router_objectpool_best_by_node_reuses", "Number of reuses in object pool best_by_node").unwrap();
    pub static ref OBJECTPOOL_BEST_PATHS_BY_NODE_NEW_ALLOCATIONS: IntCounter =
        register_int_counter!("router_objectpool_best_paths_by_node_allocations", "Number of new allocations in object pool best_paths_by_node").unwrap();
    pub static ref OBJECTPOOL_BEST_PATHS_BY_NODE_REUSES: IntCounter =
        register_int_counter!("router_objectpool_best_paths_by_node_reuses", "Number of reuses in object pool best_paths_by_node").unwrap();

    pub static ref REPRICING_DIFF_BPS: GaugeVec =
        register_gauge_vec!(opts!("router_repricing_diff_bps", "Router chaindata/live repricing diff (bps)"), &["pair"]).unwrap();

}
