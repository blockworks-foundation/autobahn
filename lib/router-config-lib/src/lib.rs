use std::{env, fs::File, io::Read};

use serde::{Deserialize, Deserializer};

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct GrpcSourceConfig {
    pub name: String,
    pub connection_string: String,
    pub token: Option<String>,
    pub retry_connection_sleep_secs: u64,
    pub tls: Option<TlsConfig>,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct QuicSourceConfig {
    pub name: String,
    #[serde(deserialize_with = "serde_string_or_env")]
    pub connection_string: String,
    pub retry_connection_sleep_secs: u64,
    pub enable_gso: Option<bool>,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct TlsConfig {
    pub ca_cert_path: String,
    pub client_cert_path: String,
    pub client_key_path: String,
    pub domain_name: String,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct Config {
    pub routing: RoutingConfig,
    pub server: ServerConfig,
    pub metrics: MetricsConfig,
    pub sources: Vec<AccountDataSourceConfig>,
    pub price_feed: PriceFeedConfig,
    pub orca: DexConfig,
    pub cropper: DexConfig,
    pub openbook_v2: DexConfig,
    pub raydium_cp: DexConfig,
    pub raydium: DexConfig,
    pub saber: DexConfig,
    pub infinity: InfinityConfig,
    pub safety_checks: Option<SafetyCheckConfig>,
    pub hot_mints: Option<HotMintsConfig>,
    pub debug_config: Option<DebugConfig>,
    pub snapshot_timeout_in_seconds: Option<u64>,
}

impl Config {
    pub fn load(path: &String) -> Result<Config, anyhow::Error> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        match toml::from_str(&contents) {
            Ok(c) => Ok(c),
            Err(e) => Err(anyhow::Error::new(e)),
        }
    }
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct HotMintsConfig {
    pub always_hot_mints: Vec<String>,
    pub keep_latest_count: usize,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct SafetyCheckConfig {
    pub check_quote_out_amount_deviation: bool,
    pub min_quote_out_to_in_amount_ratio: f64,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct DexConfig {
    pub enabled: bool,
    pub mints: Vec<String>,
    pub add_mango_tokens: bool,
    pub take_all_mints: bool,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct InfinityConfig {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct AccountDataSourceConfig {
    pub region: Option<String>,
    pub quic_sources: Option<Vec<QuicSourceConfig>>,
    #[serde(deserialize_with = "serde_string_or_env")]
    pub rpc_http_url: String,
    // does RPC node support getProgramAccountsCompressed
    pub rpc_support_compression: Option<bool>,
    pub re_snapshot_interval_secs: Option<u64>,
    pub grpc_sources: Option<Vec<GrpcSourceConfig>>,
    pub dedup_queue_size: usize,
    pub request_timeout_in_seconds: Option<u64>,
    pub number_of_accounts_per_gma: Option<usize>,
}

#[derive(Clone, Debug, serde_derive::Deserialize)]
pub enum PathWarmingMode {
    None,
    ConfiguredMints,
    MangoMints,
    All,
    HotMints,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct RoutingConfig {
    pub path_cache_validity_ms: u64,
    pub lookup_tables: Vec<String>,
    pub path_warming_interval_secs: Option<u64>,
    pub path_warming_for_mints: Option<Vec<String>>,
    pub path_warming_mode: Option<PathWarmingMode>,
    pub path_warming_amounts: Option<Vec<u64>>,
    pub path_warming_max_accounts: Option<Vec<usize>>,
    pub slot_excessive_lag: Option<u64>,
    pub slot_excessive_lag_max_duration_secs: Option<u64>,
    pub cooldown_duration_multihop_secs: Option<u64>,
    pub cooldown_duration_singlehop_secs: Option<u64>,

    /// When quoting, find best path for amount * (1 + `overquote`)
    /// So that we have best chance that this liquidity is still available when swapping
    pub overquote: Option<f64>,

    pub max_path_length: Option<usize>,
    pub retain_path_count: Option<usize>,
    pub max_edge_per_pair: Option<usize>,
    pub max_edge_per_cold_pair: Option<usize>,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct ServerConfig {
    #[serde(deserialize_with = "serde_string_or_env")]
    pub address: String,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct DebugConfig {
    pub reprice_using_live_rpc: bool,
    pub reprice_probability: f64,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct MetricsConfig {
    pub output_stdout: bool,
    pub output_http: bool,
    pub prometheus_address: Option<String>,
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize)]
pub struct PriceFeedConfig {
    #[serde(deserialize_with = "serde_string_or_env")]
    pub birdeye_token: String,
    pub birdeye_single_mode: Option<bool>,
    pub refresh_interval_secs: u64,
}

/// Get a string content, or the content of an Env variable it the string start with $
///
/// Example:
///  - "abc" -> "abc"
///  - "$something" -> read env variable named something and return it's content
///
/// *WARNING*: May kill the program if we are asking for anv environment variable that does not exist
pub fn serde_string_or_env<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value_or_env = String::deserialize(deserializer)?;
    let value = match &value_or_env.chars().next().unwrap() {
        '$' => env::var(&value_or_env[1..])
            .unwrap_or_else(|_| panic!("reading `{}` from env", &value_or_env[1..])),
        _ => value_or_env,
    };
    Ok(value)
}

/// Get a string content, or the content of an Env variable it the string start with $
///
/// Example:
///  - "abc" -> "abc"
///  - "$something" -> read env variable named something and return it's content
///
/// *WARNING*: May kill the program if we are asking for anv environment variable that does not exist
pub fn serde_opt_string_or_env<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value_or_env = Option::<String>::deserialize(deserializer)?;
    if value_or_env.is_none() {
        return Ok(None);
    }

    let value_or_env = value_or_env.unwrap();
    let value = match &value_or_env.chars().next().unwrap() {
        '$' => env::var(&value_or_env[1..]).expect("reading from env"),
        _ => value_or_env,
    };
    Ok(Some(value))
}

pub fn string_or_env(value_or_env: String) -> String {
    let value = match &value_or_env.chars().next().unwrap() {
        '$' => env::var(&value_or_env[1..]).expect("reading from env"),
        _ => value_or_env,
    };

    value
}
