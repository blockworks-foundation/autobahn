use serde::{Deserialize, Deserializer};
use std::env;

#[derive(Clone, Debug, serde_derive::Deserialize)]
pub struct Config {
    #[serde(deserialize_with = "serde_string_or_env")]
    pub router: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub owner: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub rpc_http_url: String,

    #[serde(deserialize_with = "serde_string_or_env")]
    pub outgoing_rpc_http_url: String,

    pub mints: Vec<String>,
    pub use_mango_tokens: bool,

    pub amounts: Vec<u64>,
    pub execution_interval_sec: u64,
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
        '$' => env::var(&value_or_env[1..]).expect("reading from env"),
        _ => value_or_env,
    };
    Ok(value)
}
