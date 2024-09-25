#![allow(dead_code)]

use anyhow::Context;
use itertools::Itertools;
use router_lib::mango::mango_fetcher::MangoMetadata;
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;

pub fn get_configured_mints(
    mango_metadata: &Option<MangoMetadata>,
    enabled: bool,
    add_mango_tokens: bool,
    configured_mints: &Vec<String>,
) -> anyhow::Result<HashSet<Pubkey>> {
    if !enabled {
        return Ok(HashSet::new());
    }

    let mut mints = configured_mints
        .iter()
        .map(|s| Pubkey::from_str(s).context(format!("mint {s}")))
        .collect::<anyhow::Result<Vec<Pubkey>>>()?;

    if add_mango_tokens {
        match mango_metadata.as_ref() {
            None => anyhow::bail!("Failed to init dex - missing mango metadata"),
            Some(m) => mints.extend(m.mints.clone()),
        };
    }

    let mints = mints
        .into_iter()
        .collect::<HashSet<Pubkey>>()
        .into_iter()
        .collect();

    Ok(mints)
}

// note used ATM
pub(crate) fn filter_pools_and_mints<T, F>(
    pools: Vec<(Pubkey, T)>,
    mints: &HashSet<Pubkey>,
    take_all_mints: bool,
    mints_getter: F,
) -> Vec<(Pubkey, T)>
where
    F: Fn(&T) -> (Pubkey, Pubkey),
{
    pools
        .into_iter()
        .filter(|(_pool_pk, pool)| {
            let keys = mints_getter(&pool);
            take_all_mints || mints.contains(&keys.0) && mints.contains(&keys.1)
        })
        .collect_vec()
}
