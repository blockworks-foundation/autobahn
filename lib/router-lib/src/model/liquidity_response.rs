use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde_with::serde_as]
pub struct LiquidityResponse {
    pub liquidity: HashMap<String, f64>,
}
