pub use mango_feeds_connector::chain_data::ChainData;
use std::sync::{Arc, RwLock};
pub type ChainDataArcRw = Arc<RwLock<ChainData>>;
