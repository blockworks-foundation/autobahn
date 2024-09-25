use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde_with::serde_as]
pub struct QuoteResponse {
    pub input_mint: String,
    pub in_amount: Option<String>,
    pub output_mint: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub swap_mode: String,
    pub slippage_bps: i32,
    pub platform_fee: Option<PlatformFee>,
    pub price_impact_pct: String,
    pub route_plan: Vec<RoutePlan>,
    pub accounts: Option<Vec<QuoteAccount>>,
    pub context_slot: u64,
    pub time_taken: f64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde_with::serde_as]
pub struct QuoteAccount {
    pub address: String,
    pub slot: u64,
    #[serde_as(as = "Base64")]
    pub data: Vec<u8>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlatformFee {
    pub amount: String,
    pub fee_bps: i32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RoutePlan {
    pub percent: i32,
    pub swap_info: Option<SwapInfo>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SwapInfo {
    pub amm_key: String,
    pub label: Option<String>,
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: String,
    pub out_amount: String,
    pub fee_amount: String,
    pub fee_mint: String,
}
