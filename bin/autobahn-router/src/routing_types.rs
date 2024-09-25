use crate::edge::Edge;
use mango_feeds_connector::chain_data::AccountData;
use solana_program::pubkey::Pubkey;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::sync::Arc;
use std::vec::IntoIter;
use tracing::log::trace;

/// Types

#[derive(Clone)]
pub struct RouteStep {
    pub edge: Arc<Edge>,
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
}

// no clone
pub struct Route {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub in_amount: u64,
    pub out_amount: u64,
    pub price_impact_bps: u64,
    // TODO: allow for multiple paths
    pub steps: Vec<RouteStep>,
    pub slot: u64,
    pub accounts: Option<HashMap<Pubkey, AccountData>>,
}

#[derive(Clone)]
pub(crate) struct EdgeWithNodes {
    pub(crate) source_node: MintNodeIndex,
    pub(crate) target_node: MintNodeIndex,
    pub(crate) edge: EdgeIndex,
}

#[derive(Clone)]
pub(crate) struct EdgeInfo {
    pub(crate) price: f64,
    pub(crate) accounts: usize,
}

// very special type
pub(crate) type BestVec3 = [f64; 3];

/// Mint index

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MintNodeIndex {
    idx: u32,
}

impl MintNodeIndex {
    // keep private
    fn idx(&self) -> usize {
        self.idx as usize
    }
    pub fn idx_raw(&self) -> u32 {
        self.idx
    }
}

impl From<usize> for MintNodeIndex {
    fn from(idx: usize) -> Self {
        MintNodeIndex { idx: idx as u32 }
    }
}

impl Display for MintNodeIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/mi", self.idx)
    }
}

/// Mint Vec

// indexed vector of mints
// look, ma! no Clone
pub struct MintVec<T>
where
    T: Clone,
{
    // Vec's little brother which cannot grow
    array: Box<[T]>,
}

impl<T: Clone> From<Vec<T>> for MintVec<T> {
    fn from(initial: Vec<T>) -> Self {
        MintVec {
            array: initial.into_boxed_slice(),
        }
    }
}

impl<T: Clone> MintVec<T> {
    // clone each element from the prototype
    pub fn new_from_prototype(size: usize, prototype: T) -> Self {
        trace!("init MintVec of size {}", size);
        MintVec {
            array: vec![prototype.clone(); size].into_boxed_slice(),
        }
    }

    pub fn new_from_constructor(size: usize, constructor: impl Fn() -> T) -> Self {
        trace!("init MintVec of size {}", size);
        MintVec {
            array: vec![constructor(); size].into_boxed_slice(),
        }
    }

    // copy from another MintVec without memory allocation
    pub fn try_clone_from(&mut self, other: &Self) -> bool {
        if self.array.len() != other.array.len() {
            return false;
        }
        self.array.clone_from_slice(&other.array);
        true
    }
}

impl<T: Clone> Index<MintNodeIndex> for MintVec<T> {
    type Output = T;

    fn index(&self, index: MintNodeIndex) -> &Self::Output {
        &self.array[index.idx()]
    }
}

impl<T: Clone> IndexMut<MintNodeIndex> for MintVec<T> {
    fn index_mut(&mut self, index: MintNodeIndex) -> &mut Self::Output {
        &mut self.array[index.idx()]
    }
}

impl<T: Clone> Deref for MintVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.array
    }
}

impl<T: Clone> DerefMut for MintVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.array
    }
}

impl<T: Clone> IntoIterator for MintVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.array.into_vec().into_iter()
    }
}

/// Edge index

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeIndex {
    idx: u32,
}

impl EdgeIndex {
    pub fn idx(&self) -> usize {
        self.idx as usize
    }
}

impl From<usize> for EdgeIndex {
    fn from(idx: usize) -> Self {
        EdgeIndex { idx: idx as u32 }
    }
}

impl Display for EdgeIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/ei", self.idx)
    }
}
