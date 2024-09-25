use crate::dex::DexEdgeIdentifier;
use anyhow::Context;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn http_error_handling<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let status = response.status();
    let response_text = response
        .text()
        .await
        .context("awaiting body of http request")?;
    if !status.is_success() {
        anyhow::bail!("http request failed, status: {status}, body: {response_text}");
    }
    serde_json::from_str::<T>(&response_text)
        .with_context(|| format!("response has unexpected format, body: {response_text}"))
}

pub fn insert_or_extend(
    map: &mut HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>,
    pk: &Pubkey,
    entry: &Vec<Arc<dyn DexEdgeIdentifier>>,
) {
    if let Some(old) = map.get(pk) {
        let mut extended = vec![];
        extended.extend(old.iter().map(|x| x.clone()));
        extended.extend(entry.into_iter().map(|x| x.clone()));
        map.insert(*pk, extended);
    } else {
        map.insert(*pk, entry.clone());
    }
}
