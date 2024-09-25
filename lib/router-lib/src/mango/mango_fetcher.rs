use anyhow::Context;
use router_config_lib::Config;
use serde_derive::{Deserialize, Serialize};
use solana_client::client_error::reqwest;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct MangoMetadata {
    pub mints: HashSet<Pubkey>,
    pub obv2_markets: HashSet<Pubkey>,
}

/// Return (mints, obv2-markets)
pub async fn fetch_mango_data() -> anyhow::Result<MangoMetadata> {
    let address = "https://api.mngo.cloud/data/v4/group-metadata";
    let http_client = reqwest::Client::new();
    let response = http_client
        .get(address)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .context("mango group request")?;

    let metadata: anyhow::Result<MangoGroupMetadataResponse> =
        crate::utils::http_error_handling(response).await;

    let metadata = metadata?;

    let mut mints = HashSet::new();
    let mut obv2_markets = HashSet::new();

    for group in &metadata.groups {
        for token in &group.tokens {
            mints.insert(Pubkey::from_str(token.mint.as_str())?);
        }
        for market in &group.openbook_v2_markets {
            obv2_markets.insert(Pubkey::from_str(market.serum_market_external.as_str())?);
        }
    }

    Ok(MangoMetadata {
        mints,
        obv2_markets,
    })
}

pub fn spawn_mango_watcher(
    initial_data: &Option<MangoMetadata>,
    _config: &Config,
) -> Option<JoinHandle<()>> {
    if initial_data.is_none() {
        return None;
    }
    let initial_data = initial_data.clone().unwrap();

    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 15));
        interval.tick().await;

        loop {
            interval.tick().await;

            let data = fetch_mango_data().await;
            match data {
                Ok(data) => {
                    info!(
                        tokens = data.mints.len(),
                        obv2_markets = data.obv2_markets.len(),
                        "mango metadata"
                    );

                    if data
                        .obv2_markets
                        .difference(&initial_data.obv2_markets)
                        .count()
                        > 0
                    {
                        warn!("new obv2 markets on mango");
                        break;
                    }
                    if data.mints.difference(&initial_data.mints).count() > 0 {
                        warn!("new tokens on mango");
                        break;
                    }
                }
                Err(e) => {
                    error!("Couldn't fetch mango metadata data: {}", e);
                }
            }
        }
    }))
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangoGroupMetadataResponse {
    pub groups: Vec<MangoGroupMetadata>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangoGroupMetadata {
    pub tokens: Vec<MangoGroupTokenMetadata>,
    pub openbook_v2_markets: Vec<MangoGroupObv2MarketsMetadata>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangoGroupTokenMetadata {
    pub mint: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangoGroupObv2MarketsMetadata {
    pub serum_market_external: String,
}
