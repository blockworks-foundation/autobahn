use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct LiquidityRequest {
    pub mints: String,
}
