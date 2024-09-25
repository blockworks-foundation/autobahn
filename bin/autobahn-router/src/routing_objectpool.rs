use crate::metrics;
use crate::routing_types::*;
use opool::{Pool, PoolAllocator, RefGuard};
use ordered_float::NotNan;
use router_lib::dex::SwapMode;
use tracing::{debug, trace};

pub struct RoutingObjectPools {
    best_by_node_pool: Pool<BestByNodeAllocator, Vec<BestVec3>>,
    best_paths_by_node_pool:
        Pool<BestPathsByNodeAllocator, MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>>,
    best_paths_by_node_pool_exact_out:
        Pool<BestPathsByNodeAllocator, MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>>,
    out_edges_per_node: usize,
    n_paths: usize,
}

const POOL_SIZE_BEST_BY_NODE: usize = 5;
const POOL_SIZE_BEST_PATHS_BY_NODE: usize = 5;

impl RoutingObjectPools {
    pub fn new(out_edges_per_node: usize, n_paths: usize) -> Self {
        debug!(
            "Init objectpool(size {}) for best_by_node with out_edges_per_node={}",
            POOL_SIZE_BEST_BY_NODE, out_edges_per_node
        );
        let pool_best_by_node = Pool::new_prefilled(
            POOL_SIZE_BEST_BY_NODE,
            BestByNodeAllocator { out_edges_per_node },
        );

        debug!("Init objectpool(size {}) for best_paths_by_node with out_edges_per_node={}, n_paths={}",
            POOL_SIZE_BEST_PATHS_BY_NODE, out_edges_per_node, n_paths);
        let pool_best_paths = Pool::new_prefilled(
            POOL_SIZE_BEST_PATHS_BY_NODE,
            BestPathsByNodeAllocator {
                out_edges_per_node,
                n_paths,
                swap_mode: SwapMode::ExactIn,
            },
        );

        debug!("Init objectpool(size {}) for best_paths_by_node_exact_out with out_edges_per_node={}, n_paths={}",
            POOL_SIZE_BEST_PATHS_BY_NODE, out_edges_per_node, n_paths);
        let pool_best_paths_exactout = Pool::new_prefilled(
            POOL_SIZE_BEST_PATHS_BY_NODE,
            BestPathsByNodeAllocator {
                out_edges_per_node,
                n_paths,
                swap_mode: SwapMode::ExactOut,
            },
        );

        Self {
            best_by_node_pool: pool_best_by_node,
            best_paths_by_node_pool: pool_best_paths,
            best_paths_by_node_pool_exact_out: pool_best_paths_exactout,
            out_edges_per_node,
            n_paths,
        }
    }

    /// get object from pool or create new one
    pub(crate) fn get_best_by_node(
        &self,
        expected_out_edges_per_node: usize,
    ) -> RefGuard<BestByNodeAllocator, Vec<BestVec3>> {
        assert_eq!(
            expected_out_edges_per_node, self.out_edges_per_node,
            "requested data shape does not fit the pooled vecvec"
        );
        self.best_by_node_pool.get()
    }

    /// get object from pool or create new one
    pub(crate) fn get_best_paths_by_node(
        &self,
        expected_out_edges_per_node: usize,
        expected_n_paths: usize,
    ) -> RefGuard<BestPathsByNodeAllocator, MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>> {
        assert_eq!(
            expected_out_edges_per_node, self.out_edges_per_node,
            "requested data shape does not fit the pooled one"
        );
        assert_eq!(
            expected_n_paths, self.n_paths,
            "requested data shape does not fit the pooled one"
        );
        self.best_paths_by_node_pool.get()
    }

    pub(crate) fn get_best_paths_by_node_exact_out(
        &self,
        expected_out_edges_per_node: usize,
        expected_n_paths: usize,
    ) -> RefGuard<BestPathsByNodeAllocator, MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>> {
        assert_eq!(
            expected_out_edges_per_node, self.out_edges_per_node,
            "requested data shape does not fit the pooled one"
        );
        assert_eq!(
            expected_n_paths, self.n_paths,
            "requested data shape does not fit the pooled one"
        );
        self.best_paths_by_node_pool_exact_out.get()
    }
}

pub struct BestByNodeAllocator {
    out_edges_per_node: usize,
}

impl PoolAllocator<Vec<BestVec3>> for BestByNodeAllocator {
    fn reset(&self, obj: &mut Vec<BestVec3>) {
        trace!("RESET/REUSE pooled object best_by_node");
        metrics::OBJECTPOOL_BEST_BY_NODE_REUSES.inc();
        for best_vec in obj.iter_mut() {
            best_vec.fill(0.0);
        }
    }

    /// 75MB for out_edges_per_node=393709
    #[inline]
    fn allocate(&self) -> Vec<BestVec3> {
        trace!("ALLOC bestvec object best_by_node");
        metrics::OBJECTPOOL_BEST_BY_NODE_NEW_ALLOCATIONS.inc();
        // Best amount received for token/account_size
        // 3 = number of path kept
        // 8 = (64/8) (max accounts/bucket_size)
        vec![[0f64; 3]; 8 * self.out_edges_per_node]
    }
}

pub struct BestPathsByNodeAllocator {
    out_edges_per_node: usize,
    n_paths: usize,
    swap_mode: SwapMode,
}

impl PoolAllocator<MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>> for BestPathsByNodeAllocator {
    fn reset(&self, obj: &mut MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>>) {
        trace!("RESET/REUSE pooled object best_paths_by_node");
        metrics::OBJECTPOOL_BEST_PATHS_BY_NODE_REUSES.inc();

        obj.iter_mut().for_each(|path| {
            assert_eq!(path.len(), self.n_paths);
        });

        let inf_value = match &self.swap_mode {
            SwapMode::ExactIn => f64::NEG_INFINITY,
            SwapMode::ExactOut => f64::INFINITY,
        };

        obj.iter_mut().flatten().for_each(|(ref mut top, edges)| {
            *top = NotNan::new(inf_value).unwrap();
            edges.clear();
        });
    }

    /// 72MB for out_edges_per_node=393709, n_paths=5
    #[inline]
    fn allocate(&self) -> MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>> {
        trace!("ALLOC vecvecpathedge object best_paths_by_node");
        metrics::OBJECTPOOL_BEST_PATHS_BY_NODE_NEW_ALLOCATIONS.inc();

        let inf_value = match &self.swap_mode {
            SwapMode::ExactIn => f64::NEG_INFINITY,
            SwapMode::ExactOut => f64::INFINITY,
        };

        MintVec::new_from_prototype(
            self.out_edges_per_node,
            vec![(NotNan::new(inf_value).unwrap(), vec![]); self.n_paths],
        )
    }
}

#[cfg(test)]
pub(crate) fn alloc_best_by_node_for_test(out_edges_per_node: usize) -> Vec<BestVec3> {
    vec![[0f64; 3]; 8 * out_edges_per_node]
}

#[cfg(test)]
pub(crate) fn alloc_best_paths_by_node_for_test(
    out_edges_per_node: usize,
    n_paths: usize,
    swap_mode: SwapMode,
) -> MintVec<Vec<(NotNan<f64>, Vec<EdgeWithNodes>)>> {
    let inf_value = match &swap_mode {
        SwapMode::ExactIn => f64::NEG_INFINITY,
        SwapMode::ExactOut => f64::INFINITY,
    };

    MintVec::new_from_prototype(
        out_edges_per_node,
        vec![(NotNan::new(inf_value).unwrap(), vec![]); n_paths],
    )
}
