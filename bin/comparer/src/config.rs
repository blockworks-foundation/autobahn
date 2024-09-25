use services_mango_lib::env_helper::string_or_env as serde_string_or_env;
use services_mango_lib::postgres_configuration::PostgresConfiguration;

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct Config {
    #[serde(deserialize_with = "serde_string_or_env")]
    pub rpc_http_url: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub outgoing_rpc_http_url: String,

    pub postgres: PostgresConfiguration,
    pub persist: bool,

    pub mints: Vec<String>,
    pub use_mango_tokens: bool,
    pub amounts: Vec<u64>,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub wallet_pubkey: String,

    pub execution_interval_sec: u64,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub router: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub jupiter: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub birdeye_token: String,
}
