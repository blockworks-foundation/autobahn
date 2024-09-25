use serde::Deserialize;

use crate::dex::SwapMode;

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub input_mint: String,
    pub output_mint: String,
    pub amount: u64,
    pub slippage_bps: u64,
    pub only_direct_routes: Option<bool>,
    pub max_accounts: Option<u8>,
    pub swap_mode: Option<SwapMode>,
    // mango UI uses mode and jupiter supports it so we add a support for it too.
    pub mode: Option<SwapMode>,
}
