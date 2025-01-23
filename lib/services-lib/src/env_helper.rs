use serde::{Deserialize, Deserializer};
use std::env;

/// Get a string content, or the content of an Env variable it the string start with $
///
/// Example:
///  - "abc" -> "abc"
///  - "$something" -> read env variable named something and return it's content
///
/// *WARNING*: May kill the program if we are asking for anv environment variable that does not exist
pub fn string_or_env<'de, D>(deserializer: D) -> Result<String, D::Error>
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
