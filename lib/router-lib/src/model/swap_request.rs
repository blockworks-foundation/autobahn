use crate::model::quote_response::QuoteResponse;
use serde::Deserialize;
use serde_derive::Serialize;

#[derive(Deserialize, Serialize, Debug)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct SwapForm {
    pub token: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest {
    pub user_public_key: String,
    #[serde(default = "default_true")]
    pub wrap_and_unwrap_sol: bool,
    #[serde(default = "default_true")]
    pub auto_create_out_ata: bool,
    #[serde(default)]
    pub use_shared_accounts: bool,
    pub fee_account: Option<String>,
    pub compute_unit_price_micro_lamports: Option<u64>,
    #[serde(default)]
    pub as_legacy_transaction: bool,
    #[serde(default)]
    pub use_token_ledger: bool,
    pub destination_token_account: Option<String>,
    pub quote_response: QuoteResponse,
}

fn default_true() -> bool {
    true
}
